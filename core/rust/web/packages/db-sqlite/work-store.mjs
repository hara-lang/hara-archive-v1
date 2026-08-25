import { decodeHta, encodeHta, HtaKeyword } from "@hara-lang/hta";

export const WORK_SCHEMA_VERSION = 1;

const STATUS_TRANSITIONS = Object.freeze({
  created: new Set(["created", "queued", "running", "cancelled"]),
  queued: new Set(["queued", "running", "cancelled"]),
  running: new Set(["running", "completed", "failed", "cancelled"]),
  failed: new Set(["failed", "running", "cancelled"]),
  completed: new Set(["completed"]),
  cancelled: new Set(["cancelled"])
});

function keywordName(value) {
  return value && typeof value === "object" && typeof value.name === "string"
    ? value.name
    : value;
}

function keyword(name) {
  return new HtaKeyword(name);
}

function assertMap(value, label) {
  if (!(value instanceof Map)) throw new Error(`work/store-input-invalid: ${label} must be a map`);
  return value;
}

function entryKey(map, name) {
  if (!(map instanceof Map)) return undefined;
  for (const key of map.keys()) if (keywordName(key) === name) return key;
  return undefined;
}

function get(map, name, fallback = undefined) {
  const key = entryKey(map, name);
  return key === undefined ? fallback : map.get(key);
}

function has(map, name) {
  return entryKey(map, name) !== undefined;
}

function assoc(map, name, value) {
  const output = new Map(map ?? []);
  const key = entryKey(output, name);
  output.set(key === undefined ? keyword(name) : key, value);
  return output;
}

function dissoc(map, ...names) {
  const output = new Map(map ?? []);
  for (const name of names) {
    const key = entryKey(output, name);
    if (key !== undefined) output.delete(key);
  }
  return output;
}

function merge(...maps) {
  let output = new Map();
  for (const map of maps) {
    if (!(map instanceof Map)) continue;
    for (const [key, value] of map) output = assoc(output, keywordName(key), value);
  }
  return output;
}

function result(entries) {
  return new Map(entries.map(([name, value]) => [keyword(name), value]));
}

function bytesEqual(left, right) {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
}

function equal(left, right) {
  return bytesEqual(encodeHta(left), encodeHta(right));
}

function keyOf(value) {
  return Array.from(encodeHta(value), byte => byte.toString(16).padStart(2, "0")).join("");
}

function encodeRecord(value) {
  return encodeHta(assertMap(value, "record"));
}

function decodeRecord(bytes, table, key) {
  try {
    const value = decodeHta(bytes);
    if (!(value instanceof Map)) throw new Error("decoded value is not a map");
    return value;
  } catch (error) {
    throw new Error(`work/store-corrupt: ${table}/${key}: ${error.message}`);
  }
}

function rows(database, sql, bind = []) {
  const output = [];
  database.exec({ sql, bind, rowMode: "array", resultRows: output });
  return output;
}

function row(database, sql, bind = []) {
  return rows(database, sql, bind)[0] ?? null;
}

function scalar(database, sql, bind = []) {
  return row(database, sql, bind)?.[0] ?? null;
}

function execute(database, sql, bind = []) {
  database.exec({ sql, bind });
}

function transaction(database, operation) {
  execute(database, "BEGIN IMMEDIATE");
  try {
    const value = operation();
    execute(database, "COMMIT");
    return value;
  } catch (error) {
    try {
      execute(database, "ROLLBACK");
    } catch (_) {
      // Preserve the original operational-store error.
    }
    throw error;
  }
}

function nextCounter(database, name) {
  execute(
    database,
    `INSERT INTO work_counters(name, value) VALUES (?, 1)
     ON CONFLICT(name) DO UPDATE SET value = value + 1`,
    [name]
  );
  return Number(scalar(database, "SELECT value FROM work_counters WHERE name = ?", [name]));
}

function normaliseRun(run) {
  assertMap(run, "run");
  const status = get(run, "run/status", keyword("created"));
  let output = merge(
    result([
      ["run/revision", 0],
      ["run/status", status],
      ["run/execution-status", status],
      ["run/receipt-status", keyword("pending")],
      ["run/acceptance-status", keyword("pending")]
    ]),
    run
  );
  output = assoc(output, "run/revision", Number(get(run, "run/revision", 0)));
  output = assoc(output, "run/status", status);
  return assoc(output, "run/execution-status", status);
}

function validateRunId(id) {
  if (id === null || id === undefined) throw new Error("work/run-identity-invalid: identity cannot be nil");
  return id;
}

function compatibleRun(existing, proposed) {
  return equal(get(existing, "run/work-root"), get(proposed, "run/work-root"))
    && equal(get(existing, "run/work-version"), get(proposed, "run/work-version"))
    && equal(get(existing, "run/input"), get(proposed, "run/input"));
}

function loadRun(database, id) {
  const found = row(database, "SELECT record FROM work_runs WHERE id = ?", [String(id)]);
  return found ? decodeRecord(found[0], "runs", id) : null;
}

function loadCheckpoint(database, key) {
  const storageKey = keyOf(key);
  const found = row(database, "SELECT record FROM work_checkpoints WHERE storage_key = ?", [storageKey]);
  return found ? decodeRecord(found[0], "checkpoints", storageKey) : null;
}

function loadOutbox(database, id) {
  const found = row(database, "SELECT record FROM work_outbox WHERE id = ?", [String(id)]);
  return found ? decodeRecord(found[0], "outbox", id) : null;
}

function canonicalUpdates(updates) {
  const output = new Map(updates ?? []);
  return has(output, "run/status")
    ? assoc(output, "run/execution-status", get(output, "run/status"))
    : output;
}

function assertStatusTransition(run, updates) {
  const current = String(keywordName(get(run, "run/status")));
  const proposed = String(keywordName(get(updates, "run/status", get(run, "run/status"))));
  if (!STATUS_TRANSITIONS[current]?.has(proposed)) {
    throw new Error(`work/store-status-transition: ${current} -> ${proposed}`);
  }
}

function checkpointIntent(value) {
  return dissoc(value, "checkpoint/sequence");
}

function outboxIntent(value) {
  return dissoc(
    value,
    "outbox/id",
    "outbox/sequence",
    "outbox/status",
    "outbox/claim-id",
    "outbox/claim",
    "outbox/claim-until",
    "outbox/ack"
  );
}

function applyCheckpoint(database, runId, checkpoint) {
  assertMap(checkpoint, "checkpoint");
  const checkpointRun = get(checkpoint, "checkpoint/run");
  if (checkpointRun !== undefined && !equal(checkpointRun, runId)) {
    throw new Error("work/checkpoint-run-conflict: checkpoint belongs to another run");
  }
  const key = get(checkpoint, "checkpoint/key");
  if (key === null || key === undefined) throw new Error("work/checkpoint-key-required: missing key");
  const storageKey = keyOf(key);
  const found = row(
    database,
    "SELECT status, record FROM work_checkpoints WHERE storage_key = ?",
    [storageKey]
  );
  let proposed = assoc(checkpoint, "checkpoint/run", runId);
  if (found && found[0] === "completed") {
    const existing = decodeRecord(found[1], "checkpoints", storageKey);
    if (!equal(checkpointIntent(existing), checkpointIntent(proposed))) {
      throw new Error("work/checkpoint-immutable: completed checkpoint differs");
    }
    return existing;
  }
  const sequence = nextCounter(database, "checkpoint-sequence");
  proposed = assoc(proposed, "checkpoint/sequence", sequence);
  execute(
    database,
    `INSERT INTO work_checkpoints(storage_key, run_id, status, sequence, record)
     VALUES (?, ?, ?, ?, ?)
     ON CONFLICT(storage_key) DO UPDATE SET
       run_id = excluded.run_id,
       status = excluded.status,
       sequence = excluded.sequence,
       record = excluded.record`,
    [
      storageKey,
      String(runId),
      String(keywordName(get(proposed, "checkpoint/status", keyword("pending")))),
      sequence,
      encodeRecord(proposed)
    ]
  );
  return proposed;
}

function applyEvent(database, runId, event) {
  assertMap(event, "event");
  const sequence = nextCounter(database, "event-sequence");
  let record = assoc(event, "run/id", runId);
  record = assoc(record, "event/sequence", sequence);
  execute(
    database,
    "INSERT INTO work_events(sequence, run_id, record) VALUES (?, ?, ?)",
    [sequence, String(runId), encodeRecord(record)]
  );
  return record;
}

function applyOutbox(database, runId, entry) {
  assertMap(entry, "outbox entry");
  const key = get(entry, "outbox/key");
  if (key === null || key === undefined) throw new Error("work/outbox-key-required: missing key");
  const storageKey = keyOf(key);
  let proposed = merge(
    result([["outbox/run", runId], ["outbox/status", keyword("pending")]]),
    entry
  );
  proposed = assoc(proposed, "outbox/run", runId);
  const found = row(database, "SELECT id, record FROM work_outbox WHERE storage_key = ?", [storageKey]);
  if (found) {
    const existing = decodeRecord(found[1], "outbox", found[0]);
    if (!equal(outboxIntent(existing), outboxIntent(proposed))) {
      throw new Error("work/outbox-key-conflict: existing intent differs");
    }
    return existing;
  }
  const sequence = nextCounter(database, "outbox-sequence");
  const id = get(proposed, "outbox/id", `outbox-${sequence}`);
  proposed = assoc(assoc(proposed, "outbox/id", id), "outbox/sequence", sequence);
  execute(
    database,
    `INSERT INTO work_outbox(id, storage_key, run_id, status, sequence, record)
     VALUES (?, ?, ?, 'pending', ?, ?)`,
    [String(id), storageKey, String(runId), sequence, encodeRecord(proposed)]
  );
  execute(
    database,
    "INSERT INTO work_idempotency(kind, identity, record) VALUES ('outbox', ?, ?)",
    [storageKey, encodeRecord(proposed)]
  );
  return proposed;
}

function migrate(database) {
  execute(database, "PRAGMA foreign_keys = ON");
  execute(database, "PRAGMA journal_mode = DELETE");
  execute(database, "PRAGMA synchronous = FULL");
  execute(database, "PRAGMA busy_timeout = 5000");
  execute(
    database,
    `CREATE TABLE IF NOT EXISTS work_schema_versions (
       version INTEGER PRIMARY KEY,
       applied_at INTEGER NOT NULL
     );
     CREATE TABLE IF NOT EXISTS work_counters (
       name TEXT PRIMARY KEY,
       value INTEGER NOT NULL
     );
     CREATE TABLE IF NOT EXISTS work_runs (
       id TEXT PRIMARY KEY,
       status TEXT NOT NULL,
       revision INTEGER NOT NULL,
       record BLOB NOT NULL
     );
     CREATE INDEX IF NOT EXISTS work_runs_status_idx ON work_runs(status, id);
     CREATE TABLE IF NOT EXISTS work_transitions (
       run_id TEXT NOT NULL,
       revision INTEGER NOT NULL,
       record BLOB NOT NULL,
       PRIMARY KEY(run_id, revision),
       FOREIGN KEY(run_id) REFERENCES work_runs(id)
     );
     CREATE TABLE IF NOT EXISTS work_checkpoints (
       storage_key TEXT PRIMARY KEY,
       run_id TEXT NOT NULL,
       status TEXT NOT NULL,
       sequence INTEGER NOT NULL,
       record BLOB NOT NULL,
       FOREIGN KEY(run_id) REFERENCES work_runs(id)
     );
     CREATE INDEX IF NOT EXISTS work_checkpoints_run_idx ON work_checkpoints(run_id, sequence);
     CREATE TABLE IF NOT EXISTS work_events (
       sequence INTEGER PRIMARY KEY,
       run_id TEXT NOT NULL,
       record BLOB NOT NULL,
       FOREIGN KEY(run_id) REFERENCES work_runs(id)
     );
     CREATE INDEX IF NOT EXISTS work_events_run_idx ON work_events(run_id, sequence);
     CREATE TABLE IF NOT EXISTS work_outbox (
       id TEXT PRIMARY KEY,
       storage_key TEXT NOT NULL UNIQUE,
       run_id TEXT NOT NULL,
       status TEXT NOT NULL,
       sequence INTEGER NOT NULL UNIQUE,
       claim_id TEXT,
       claim_until INTEGER,
       record BLOB NOT NULL,
       FOREIGN KEY(run_id) REFERENCES work_runs(id)
     );
     CREATE INDEX IF NOT EXISTS work_outbox_select_idx ON work_outbox(status, run_id, sequence);
     CREATE TABLE IF NOT EXISTS work_idempotency (
       kind TEXT NOT NULL,
       identity TEXT NOT NULL,
       record BLOB NOT NULL,
       PRIMARY KEY(kind, identity)
     );
     CREATE TABLE IF NOT EXISTS work_leases (
       resource TEXT PRIMARY KEY,
       owner TEXT NOT NULL,
       generation INTEGER NOT NULL,
       expires_at INTEGER NOT NULL,
       record BLOB
     );
     CREATE TABLE IF NOT EXISTS work_dead_letters (
       id TEXT PRIMARY KEY,
       source TEXT NOT NULL,
       failed_at INTEGER NOT NULL,
       record BLOB NOT NULL
     );`
  );
  const version = Number(scalar(database, "SELECT COALESCE(MAX(version), 0) FROM work_schema_versions"));
  if (version > WORK_SCHEMA_VERSION) {
    throw new Error(`work/store-schema-future: ${version}`);
  }
  if (version < 1) {
    execute(
      database,
      "INSERT INTO work_schema_versions(version, applied_at) VALUES (?, ?)",
      [1, Date.now()]
    );
  }
  return result([["schema/version", WORK_SCHEMA_VERSION]]);
}

function nextRunId(database) {
  return transaction(database, () => {
    while (true) {
      const counter = nextCounter(database, "run-counter");
      const id = `run-${counter}`;
      if (!loadRun(database, id)) return id;
    }
  });
}

function createRun(database, input) {
  return transaction(database, () => {
    const run = normaliseRun(input);
    const id = validateRunId(get(run, "run/id"));
    const existing = loadRun(database, id);
    if (existing) {
      if (compatibleRun(existing, run)) return existing;
      throw new Error("work/run-identity-conflict: existing input differs");
    }
    execute(
      database,
      "INSERT INTO work_runs(id, status, revision, record) VALUES (?, ?, ?, ?)",
      [
        String(id),
        String(keywordName(get(run, "run/status"))),
        Number(get(run, "run/revision", 0)),
        encodeRecord(run)
      ]
    );
    execute(
      database,
      "INSERT INTO work_idempotency(kind, identity, record) VALUES ('run', ?, ?)",
      [String(id), encodeRecord(run)]
    );
    return run;
  });
}

function transact(database, request) {
  assertMap(request, "transition");
  return transaction(database, () => {
    const runId = validateRunId(get(request, "transition/run-id"));
    const run = loadRun(database, runId);
    if (!run) throw new Error(`work/run-missing: ${runId}`);
    const expected = Number(get(request, "transition/expected-revision"));
    const actual = Number(get(run, "run/revision", 0));
    if (expected !== actual) {
      throw new Error(`work/store-revision-conflict: expected ${expected}, actual ${actual}`);
    }
    const updates = canonicalUpdates(get(request, "transition/run-updates", new Map()));
    assertStatusTransition(run, updates);
    const revision = actual + 1;
    let committedRun = merge(run, updates);
    committedRun = assoc(committedRun, "run/revision", revision);
    const checkpoints = Array.from(get(request, "transition/checkpoints", []), value =>
      applyCheckpoint(database, runId, value)
    );
    const events = Array.from(get(request, "transition/events", []), value =>
      applyEvent(database, runId, value)
    );
    const outbox = Array.from(get(request, "transition/outbox", []), value =>
      applyOutbox(database, runId, value)
    );
    execute(
      database,
      "UPDATE work_runs SET status = ?, revision = ?, record = ? WHERE id = ?",
      [
        String(keywordName(get(committedRun, "run/status"))),
        revision,
        encodeRecord(committedRun),
        String(runId)
      ]
    );
    const response = result([
      ["transition/run", committedRun],
      ["transition/revision", revision],
      ["transition/checkpoints", checkpoints],
      ["transition/events", events],
      ["transition/outbox", outbox],
      ["transition/event-sequences", events.map(value => get(value, "event/sequence"))],
      ["transition/outbox-sequences", outbox.map(value => get(value, "outbox/sequence"))]
    ]);
    execute(
      database,
      "INSERT INTO work_transitions(run_id, revision, record) VALUES (?, ?, ?)",
      [String(runId), revision, encodeRecord(response)]
    );
    return response;
  });
}

function listOutbox(database, query) {
  const runId = get(query, "run/id");
  const status = get(query, "status");
  const clauses = [];
  const bind = [];
  if (runId !== undefined && runId !== null) {
    clauses.push("run_id = ?");
    bind.push(String(runId));
  }
  if (status !== undefined && status !== null) {
    clauses.push("status = ?");
    bind.push(String(keywordName(status)));
  }
  const where = clauses.length ? ` WHERE ${clauses.join(" AND ")}` : "";
  return rows(database, `SELECT id, record FROM work_outbox${where} ORDER BY sequence`, bind)
    .map(([id, record]) => decodeRecord(record, "outbox", id));
}

function claimOutbox(database, options) {
  assertMap(options, "claim options");
  return transaction(database, () => {
    const claimId = get(options, "claim/id");
    if (claimId === null || claimId === undefined) throw new Error("work/outbox-claim-required: missing claim/id");
    const runId = get(options, "run/id");
    const limit = Math.max(0, Number(get(options, "limit", 100)));
    const now = Number(get(options, "claim/now", Date.now()));
    const leaseMs = Number(get(options, "claim/lease-ms", 30000));
    const runClause = runId === undefined || runId === null ? "" : " AND run_id = ?";
    const runBind = runClause ? [String(runId)] : [];
    const existing = rows(
      database,
      `SELECT id, record FROM work_outbox
       WHERE status = 'claimed' AND claim_id = ?${runClause}
       ORDER BY sequence LIMIT ?`,
      [String(claimId), ...runBind, limit]
    ).map(([id, record]) => decodeRecord(record, "outbox", id));
    if (existing.length >= limit) return existing;
    const available = rows(
      database,
      `SELECT id, record FROM work_outbox
       WHERE (status = 'pending' OR (status = 'claimed' AND claim_until <= ?))${runClause}
       ORDER BY sequence LIMIT ?`,
      [now, ...runBind, limit - existing.length]
    );
    const claimed = available.map(([id, bytes]) => {
      let record = decodeRecord(bytes, "outbox", id);
      record = assoc(record, "outbox/status", keyword("claimed"));
      record = assoc(record, "outbox/claim-id", claimId);
      record = assoc(record, "outbox/claim", get(options, "claim/data", new Map()));
      record = assoc(record, "outbox/claim-until", now + leaseMs);
      execute(
        database,
        `UPDATE work_outbox
         SET status = 'claimed', claim_id = ?, claim_until = ?, record = ?
         WHERE id = ?`,
        [String(claimId), now + leaseMs, encodeRecord(record), String(id)]
      );
      return record;
    });
    return [...existing, ...claimed];
  });
}

function ackOutbox(database, id, options) {
  assertMap(options, "ack options");
  return transaction(database, () => {
    let record = loadOutbox(database, id);
    if (!record) throw new Error(`work/outbox-missing: ${id}`);
    const claimId = get(options, "claim/id");
    const status = String(keywordName(get(record, "outbox/status")));
    if (status === "acked") {
      if (equal(get(record, "outbox/claim-id"), claimId)) return record;
      throw new Error("work/outbox-ack-conflict: already acknowledged by another claim");
    }
    if (status !== "claimed" || !equal(get(record, "outbox/claim-id"), claimId)) {
      throw new Error("work/outbox-claim-conflict: acknowledgement is stale");
    }
    record = assoc(record, "outbox/status", keyword("acked"));
    record = assoc(record, "outbox/ack", get(options, "ack/data", new Map()));
    execute(
      database,
      "UPDATE work_outbox SET status = 'acked', claim_until = NULL, record = ? WHERE id = ?",
      [encodeRecord(record), String(id)]
    );
    return record;
  });
}

export function workCall(database, operationValue, argsValue) {
  const operation = String(keywordName(operationValue));
  const args = Array.from(argsValue ?? []);
  switch (operation) {
    case "migrate": return migrate(database);
    case "next-run-id": return nextRunId(database);
    case "validate-run-id": return validateRunId(args[0]);
    case "create-run": return createRun(database, args[0]);
    case "load-run": return loadRun(database, args[0]);
    case "list-runs": {
      const query = args[0] ?? new Map();
      const status = get(query, "status");
      const bind = status === undefined || status === null ? [] : [String(keywordName(status))];
      const where = bind.length ? " WHERE status = ?" : "";
      return rows(database, `SELECT id, record FROM work_runs${where} ORDER BY id`, bind)
        .map(([id, record]) => decodeRecord(record, "runs", id));
    }
    case "transact": return transact(database, args[0]);
    case "list-events":
      return rows(database, "SELECT sequence, record FROM work_events WHERE run_id = ? ORDER BY sequence", [String(args[0])])
        .map(([sequence, record]) => decodeRecord(record, "events", sequence));
    case "load-checkpoint": return loadCheckpoint(database, args[0]);
    case "list-checkpoints": {
      const query = args[0];
      const runId = query instanceof Map ? get(query, "run/id") : query;
      const bind = runId === undefined || runId === null ? [] : [String(runId)];
      const where = bind.length ? " WHERE run_id = ?" : "";
      return rows(database, `SELECT storage_key, record FROM work_checkpoints${where} ORDER BY sequence`, bind)
        .map(([storageKey, record]) => decodeRecord(record, "checkpoints", storageKey));
    }
    case "list-outbox": return listOutbox(database, args[0] ?? new Map());
    case "claim-outbox": return claimOutbox(database, args[0]);
    case "ack-outbox": return ackOutbox(database, args[0], args[1]);
    default: throw new Error(`work/store-operation-unknown: ${operation}`);
  }
}

export function mutatesWorkStore(operationValue) {
  return new Set(["migrate", "next-run-id", "create-run", "transact", "claim-outbox", "ack-outbox"])
    .has(String(keywordName(operationValue)));
}
