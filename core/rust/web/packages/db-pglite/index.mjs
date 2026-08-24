function keywordName(value) {
  return value && typeof value === "object" && typeof value.name === "string"
    ? value.name
    : value;
}

function option(options, name, fallback = undefined) {
  if (!(options instanceof Map)) return fallback;
  for (const [key, value] of options) {
    if (keywordName(key) === name) return value;
  }
  return fallback;
}

function fromHta(value) {
  if (Array.isArray(value)) return value.map(fromHta);
  if (value instanceof Uint8Array) return value;
  if (value && typeof value === "object" && Array.isArray(value.values)) {
    return value.values.map(fromHta);
  }
  if (value instanceof Map) {
    const output = Object.create(null);
    for (const [key, item] of value) {
      output[String(keywordName(key))] = fromHta(item);
    }
    return output;
  }
  const keyword = keywordName(value);
  return keyword === value ? value : keyword;
}

const OID = Object.freeze({
  bool: 16, bytea: 17, int8: 20, int2: 21, int4: 23, text: 25,
  json: 114, float4: 700, float8: 701, bpchar: 1042, varchar: 1043,
  date: 1082, time: 1083, timestamp: 1114, timestamptz: 1184,
  numeric: 1700, uuid: 2950, jsonb: 3802
});

const ARRAY_OIDS = new Map([
  [1000, [OID.bool, "bool"]], [1001, [OID.bytea, "bytea"]],
  [1005, [OID.int2, "int2"]], [1007, [OID.int4, "int4"]],
  [1009, [OID.text, "text"]], [1014, [OID.bpchar, "bpchar"]],
  [1015, [OID.varchar, "varchar"]], [1016, [OID.int8, "int8"]],
  [1021, [OID.float4, "float4"]], [1022, [OID.float8, "float8"]],
  [1115, [OID.timestamp, "timestamp"]], [1182, [OID.date, "date"]],
  [1183, [OID.time, "time"]], [1185, [OID.timestamptz, "timestamptz"]],
  [1231, [OID.numeric, "numeric"]], [199, [OID.json, "json"]],
  [2951, [OID.uuid, "uuid"]], [3807, [OID.jsonb, "jsonb"]]
]);
const TYPE_OIDS = new Map([
  ["bool", 1000], ["boolean", 1000], ["bytea", 1001], ["bytes", 1001],
  ["int2", 1005], ["smallint", 1005], ["int4", 1007], ["int", 1007],
  ["integer", 1007], ["text", 1009], ["bpchar", 1014], ["char", 1014],
  ["varchar", 1015], ["int8", 1016], ["long", 1016], ["bigint", 1016],
  ["float4", 1021], ["real", 1021], ["float8", 1022], ["double", 1022],
  ["double-precision", 1022], ["timestamp", 1115], ["date", 1182],
  ["time", 1183], ["timestamptz", 1185], ["numeric", 1231],
  ["decimal", 1231], ["json", 199], ["uuid", 2951], ["jsonb", 3807]
]);
const RAW = Symbol("postgres-raw");

function postgresTag(value) {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value.$postgres
    : undefined;
}

function numericTag(value) {
  return { $postgres: "numeric", value: String(value) };
}

function arrayTag(element, dimensions, value) {
  return { $postgres: "array", element, dimensions, value };
}

function simpleNumeric(text) {
  const value = String(text);
  if (!/^(?:NaN|-?Infinity)$/.test(value)) {
    const integral = /^([+-]?\d+)(?:\.0*)?$/.exec(value);
    if (integral) {
      try {
        const integer = BigInt(integral[1]);
        if (integer >= -(1n << 63n) && integer <= (1n << 63n) - 1n) {
          return integer >= BigInt(Number.MIN_SAFE_INTEGER) && integer <= BigInt(Number.MAX_SAFE_INTEGER)
            ? Number(integer)
            : integer;
        }
      } catch (_) {}
    }
    const number = Number(value);
    if (Number.isFinite(number)) return number;
  }
  return numericTag(value);
}

function scalarParser(oid, value) {
  if (value === null) return null;
  if (oid === OID.bool) return value === "t" || value === "true";
  if (oid === OID.int2 || oid === OID.int4) return Number(value);
  if (oid === OID.int8) {
    const integer = BigInt(value);
    return integer >= BigInt(Number.MIN_SAFE_INTEGER) && integer <= BigInt(Number.MAX_SAFE_INTEGER)
      ? Number(integer)
      : integer;
  }
  if (oid === OID.float4 || oid === OID.float8) return Number(value);
  if (oid === OID.numeric) return { [RAW]: "numeric", value };
  if (oid === OID.json || oid === OID.jsonb) return JSON.parse(value);
  if (oid === OID.bytea && /^\\x[0-9a-f]*$/i.test(value)) {
    return Uint8Array.from(value.slice(2).match(/../g) ?? [], byte => Number.parseInt(byte, 16));
  }
  return value;
}

function inferDimensions(value, output = []) {
  if (!Array.isArray(value)) return output;
  output.push([1, value.length]);
  if (value.length) inferDimensions(value[0], output);
  return output;
}

function parseArray(source, elementOid) {
  let cursor = 0;
  const dimensions = [];
  while (source[cursor] === "[") {
    const end = source.indexOf("]", cursor);
    const match = /^(-?\d+):(-?\d+)$/.exec(source.slice(cursor + 1, end));
    if (end < 0 || !match) throw new Error("postgres/query: malformed ARRAY dimensions");
    const lower = Number(match[1]);
    dimensions.push([lower, Number(match[2]) - lower + 1]);
    cursor = end + 1;
  }
  if (dimensions.length) {
    if (source[cursor] !== "=") throw new Error("postgres/query: malformed ARRAY bounds");
    cursor += 1;
  }
  function sequence() {
    if (source[cursor++] !== "{") throw new Error("postgres/query: malformed ARRAY value");
    const values = [];
    while (source[cursor] !== "}") {
      if (source[cursor] === "{") {
        values.push(sequence());
      } else {
        let value = "";
        let quoted = false;
        if (source[cursor] === '"') {
          quoted = true;
          cursor += 1;
          while (cursor < source.length && source[cursor] !== '"') {
            if (source[cursor] === "\\") cursor += 1;
            value += source[cursor++];
          }
          if (source[cursor++] !== '"') throw new Error("postgres/query: unterminated ARRAY string");
        } else {
          while (cursor < source.length && source[cursor] !== "," && source[cursor] !== "}") {
            if (source[cursor] === "\\") cursor += 1;
            value += source[cursor++];
          }
        }
        values.push(!quoted && value === "NULL" ? null : scalarParser(elementOid, value));
      }
      if (source[cursor] === ",") cursor += 1;
      else if (source[cursor] !== "}") throw new Error("postgres/query: malformed ARRAY separator");
    }
    cursor += 1;
    return values;
  }
  const value = sequence();
  if (cursor !== source.length) throw new Error("postgres/query: trailing ARRAY data");
  return {
    [RAW]: "array",
    dimensions: dimensions.length ? dimensions : value.length ? inferDimensions(value) : [],
    value
  };
}

const RESULT_PARSERS = Object.freeze(Object.fromEntries([
  [OID.numeric, value => ({ [RAW]: "numeric", value })],
  ...[...ARRAY_OIDS].map(([oid, [elementOid]]) => [oid, value => parseArray(value, elementOid)])
]));

function decodeValue(value, oid, mode) {
  if (value && value[RAW] === "numeric") {
    return mode === "tagged" ? numericTag(value.value) : simpleNumeric(value.value);
  }
  if (value && value[RAW] === "array") {
    const [elementOid, element] = ARRAY_OIDS.get(oid);
    const convert = item => Array.isArray(item)
      ? item.map(convert)
      : item && item[RAW] === "numeric"
        ? (mode === "tagged" ? numericTag(item.value) : simpleNumeric(item.value))
        : item;
    const converted = convert(value.value);
    return mode === "tagged" || value.dimensions.some(([lower]) => lower !== 1)
      ? arrayTag(element, value.dimensions, converted)
      : converted;
  }
  return value;
}

function resultShape(result, mode = "simple") {
  return {
    columns: (result.fields ?? []).map(field => field.name),
    rows: (result.rows ?? []).map(row => row.map((value, index) =>
      decodeValue(value, result.fields[index].dataTypeID, mode))),
    affected: result.affectedRows ?? 0
  };
}

function quoteArrayText(value) {
  return `"${String(value).replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`;
}

function arrayElementText(value, elementOid) {
  if (value === null || value === undefined) return "NULL";
  if (postgresTag(value) === "numeric") value = value.value;
  if (elementOid === OID.json || elementOid === OID.jsonb) return quoteArrayText(JSON.stringify(value));
  if (elementOid === OID.bytea && value instanceof Uint8Array) {
    return quoteArrayText(`\\x${[...value].map(byte => byte.toString(16).padStart(2, "0")).join("")}`);
  }
  if ([OID.text, OID.varchar, OID.bpchar, OID.uuid, OID.date, OID.time, OID.timestamp, OID.timestamptz].includes(elementOid)) {
    return quoteArrayText(value);
  }
  return String(value);
}

function arrayLiteral(value, elementOid) {
  if (!Array.isArray(value)) throw new Error("postgres/config-invalid: array value must be a vector");
  return `{${value.map(item => Array.isArray(item)
    ? arrayLiteral(item, elementOid)
    : arrayElementText(item, elementOid)).join(",")}}`;
}

function arrayParameterText(tag, arrayOid) {
  const [elementOid] = ARRAY_OIDS.get(arrayOid);
  const dimensions = tag.dimensions ?? inferDimensions(tag.value);
  const shape = inferDimensions(tag.value);
  if (dimensions.length !== shape.length || dimensions.some((dimension, index) =>
    !Array.isArray(dimension) || dimension.length !== 2 || Number(dimension[1]) !== shape[index][1])) {
    throw new Error("postgres/config-invalid: array dimensions do not match value");
  }
  const prefix = dimensions.some(([lower]) => Number(lower) !== 1)
    ? `${dimensions.map(([lower, length]) => `[${Number(lower)}:${Number(lower) + Number(length) - 1}]`).join("")}=`
    : "";
  return prefix + arrayLiteral(tag.value, elementOid);
}

async function typedParameters(database, sql, parameters) {
  const values = fromHta(parameters ?? []);
  const description = await database.describeQuery(String(sql));
  const paramTypes = description.queryParams.map(parameter => parameter.dataTypeID);
  const serializers = Object.create(null);
  for (let index = 0; index < values.length; index += 1) {
    const tag = postgresTag(values[index]);
    if (!tag || paramTypes[index] === OID.json || paramTypes[index] === OID.jsonb) continue;
    if (tag === "numeric") {
      if (paramTypes[index] !== OID.text && paramTypes[index] !== OID.numeric) {
        throw new Error(`postgres/type-unsupported: numeric parameter for oid ${paramTypes[index]}`);
      }
      paramTypes[index] = OID.numeric;
      values[index] = values[index].value;
      serializers[OID.numeric] = value => String(value);
      continue;
    }
    if (tag === "array") {
      const arrayOid = TYPE_OIDS.get(String(values[index].element));
      if (!arrayOid) throw new Error(`postgres/type-unsupported: ${values[index].element}`);
      if (paramTypes[index] !== OID.text && !ARRAY_OIDS.has(paramTypes[index])) {
        throw new Error(`postgres/type-unsupported: array parameter for oid ${paramTypes[index]}`);
      }
      if (ARRAY_OIDS.has(paramTypes[index]) && paramTypes[index] !== arrayOid) {
        throw new Error("postgres/type-unsupported: array element type mismatch");
      }
      const tagged = values[index];
      paramTypes[index] = arrayOid;
      values[index] = arrayParameterText(tagged, arrayOid);
      serializers[arrayOid] = value => typeof value === "string"
        ? value
        : arrayLiteral(value, ARRAY_OIDS.get(arrayOid)[0]);
    }
  }
  return { values, paramTypes, serializers };
}

export function createPgliteProvider(PGlite) {
  let nextConnectionId = 0;
  let nextSubscriptionId = 0;
  const connections = new Map();
  const subscriptions = new Map();

  function connection(id) {
    const database = connections.get(Number(id));
    if (!database) {
      throw new Error(`db/pglite-connection-missing: ${id}`);
    }
    return database;
  }

  async function openDatabase(options) {
    const storage = keywordName(option(options, "storage", "memory"));
    const databaseName = String(option(options, "database-name", "hestia"));
    const path = option(options, "path");
    if (!["memory", "transient", "indexeddb", "filesystem"].includes(storage)) {
      throw new Error(
        `postgres/capability-unsupported: pglite storage ${storage}`
      );
    }
    if (storage === "indexeddb" && typeof indexedDB === "undefined") {
      throw new Error("postgres/capability-unsupported: indexeddb");
    }
    const dataDir = storage === "indexeddb"
      ? `idb://${databaseName}`
      : storage === "filesystem"
        ? String(path ?? databaseName)
        : "memory://";
    const database = await PGlite.create(dataDir);
    const id = ++nextConnectionId;
    connections.set(id, database);
    return {
      id,
      engine: "postgresql",
      provider: "pglite",
      mode: "embedded",
      storage: storage === "transient" ? "memory" : storage,
      capabilities: ["sql", "transactions", "notifications", "embedded", "numeric", "arrays", "tagged-decode"]
    };
  }

  async function execute(id, sql, parameters, options) {
    const database = connection(id);
    const typed = await typedParameters(database, sql, parameters);
    const decode = String(keywordName(option(options, "decode", "simple")));
    if (decode !== "simple" && decode !== "tagged") {
      throw new Error("postgres/config-invalid: decode must be simple or tagged");
    }
    const result = await database.query(
      String(sql),
      typed.values,
      {
        rowMode: "array",
        paramTypes: typed.paramTypes,
        serializers: typed.serializers,
        parsers: RESULT_PARSERS
      }
    );
    return resultShape(result, decode);
  }

  async function closeDatabase(id) {
    const key = Number(id);
    const database = connections.get(key);
    if (!database) return false;
    for (const [subscriptionId, subscription] of subscriptions) {
      if (subscription.connection === key) await unlisten(subscriptionId);
    }
    connections.delete(key);
    await database.close();
    return true;
  }

  async function listen(id, channel) {
    const database = connection(id);
    const subscriptionId = ++nextSubscriptionId;
    const subscription = {
      id: subscriptionId,
      connection: Number(id),
      channel: String(channel),
      queue: [],
      waiters: [],
      unsubscribe: null
    };
    subscription.unsubscribe = await database.listen(subscription.channel, payload => {
      const event = { channel: subscription.channel, payload: String(payload), pid: null };
      const waiter = subscription.waiters.shift();
      if (waiter) waiter(event);
      else subscription.queue.push(event);
    });
    subscriptions.set(subscriptionId, subscription);
    return { id: subscriptionId, channel: subscription.channel };
  }

  async function nextNotification(id) {
    const subscription = subscriptions.get(Number(id));
    if (!subscription) throw new Error(`postgres/subscription-closed: ${id}`);
    if (subscription.queue.length) return subscription.queue.shift();
    return new Promise(resolve => subscription.waiters.push(resolve));
  }

  async function unlisten(id) {
    const key = Number(id);
    const subscription = subscriptions.get(key);
    if (!subscription) return false;
    subscriptions.delete(key);
    await subscription.unsubscribe();
    for (const waiter of subscription.waiters) {
      waiter({ channel: subscription.channel, closed: true });
    }
    return true;
  }

  async function notify(id, channel, payload) {
    const database = connection(id);
    await database.query("select pg_notify($1, $2)", [String(channel), String(payload)]);
    return true;
  }

  function unsupported(operation) {
    throw new Error(`postgres/capability-unsupported: ${operation}`);
  }

  async function engineVersion() {
    const database = await PGlite.create("memory://");
    try {
      const result = await database.query("select version()", [], { rowMode: "array" });
      return {
        engine: "postgresql",
        provider: "pglite",
        version: result.rows[0][0]
      };
    } finally {
      await database.close();
    }
  }

  async function call(environment, operation, args) {
    switch (operation) {
      case "describe":
        return {
          engine: "postgresql",
          provider: "pglite",
          mode: "embedded",
          capabilities: ["sql", "transactions", "notifications", "embedded", "numeric", "arrays", "tagged-decode"]
        };
      case "version":
        if (args.length) {
          const result = await connection(args[0]).query("select version()", [], { rowMode: "array" });
          return { engine: "postgresql", provider: "pglite", version: result.rows[0][0] };
        }
        return engineVersion();
      case "open":
        return openDatabase(args[0]);
      case "exec":
        return execute(args[0], args[1], args[2], null);
      case "query":
        return execute(args[0], args[1], args[2], args[3]);
      case "query-options":
        return execute(args[0], args[1], args[2], args[3]);
      case "close":
        return closeDatabase(args[0]);
      case "wait-ready":
        return { ready: true, provider: "pglite" };
      case "listen":
        return listen(args[0], args[1]);
      case "notification-next":
        return nextNotification(args[0]);
      case "unlisten":
        return unlisten(args[0]);
      case "notify":
        return notify(args[0], args[1], args[2]);
      case "database-create":
      case "database-drop":
      case "server-start":
      case "server-stop":
        return unsupported(operation);
      default:
        throw new Error(`postgres/operation-unknown: ${operation}`);
    }
  }

  async function closeAll() {
    for (const [id] of subscriptions) await unlisten(id);
    for (const [id] of connections) await closeDatabase(id);
  }

  return Object.freeze({ call, closeAll });
}
