import { mutatesWorkStore, workCall } from "./work-store.mjs";

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

export function createSqliteProvider(sqlite3InitModule, providerOptions = {}) {
  let sqlitePromise;
  let opfsPoolPromise;
  let nextConnectionId = 0;
  const connections = new Map();

  async function sqlite() {
    if (!sqlitePromise) sqlitePromise = sqlite3InitModule();
    return sqlitePromise;
  }

  function connection(id) {
    const value = connections.get(Number(id));
    if (!value) {
      throw new Error(`db/sqlite-connection-missing: ${id}`);
    }
    return value;
  }

  function deserialize(sqlite3, bytes) {
    const database = new sqlite3.oo1.DB(":memory:", "ct");
    if (!bytes || bytes.length === 0) return database;
    const pointer = sqlite3.wasm.allocFromTypedArray(bytes);
    const flags = sqlite3.capi.SQLITE_DESERIALIZE_FREEONCLOSE
      | sqlite3.capi.SQLITE_DESERIALIZE_RESIZEABLE;
    const code = sqlite3.capi.sqlite3_deserialize(
      database.pointer,
      "main",
      pointer,
      bytes.length,
      bytes.length,
      flags
    );
    if (code !== 0) {
      sqlite3.wasm.dealloc(pointer);
      database.close();
      throw new Error(`db/sqlite-deserialize: SQLite result ${code}`);
    }
    return database;
  }

  function exportDatabase(value) {
    return value.sqlite3.capi.sqlite3_js_db_export(value.database.pointer);
  }

  async function opfsPool(sqlite3) {
    if (!opfsPoolPromise) {
      if (typeof sqlite3.installOpfsSAHPoolVfs !== "function") {
        throw new Error("db/sqlite-opfs-unavailable: SAH pool VFS is not installed");
      }
      opfsPoolPromise = sqlite3.installOpfsSAHPoolVfs({ initialCapacity: 6 });
    }
    return opfsPoolPromise;
  }

  async function acquireOpfsLease(path) {
    const locks = globalThis.navigator?.locks;
    if (!locks || typeof locks.request !== "function") {
      throw new Error("db/sqlite-opfs-lock-unavailable: Web Locks are required for OPFS storage");
    }
    let release;
    let acquired;
    let rejectAcquired;
    const ready = new Promise((resolve, reject) => {
      acquired = resolve;
      rejectAcquired = reject;
    });
    const hold = new Promise(resolve => {
      release = resolve;
    });
    let released = false;
    let entered = false;
    const request = locks.request(
      `hara/sqlite/opfs:${path}`,
      { mode: "exclusive" },
      async lock => {
        if (!lock) throw new Error("db/sqlite-opfs-lock-denied: exclusive lease was not granted");
        entered = true;
        acquired();
        await hold;
      }
    );
    request.catch(error => {
      if (!entered) rejectAcquired(error);
    });
    try {
      await new Promise((resolve, reject) => {
        const resolveReady = () => resolve();
        const rejectReady = error => reject(error);
        ready.then(resolveReady, rejectReady);
      });
    } catch (error) {
      throw new Error(`db/sqlite-opfs-lock-failed: ${error.message}`);
    }
    return Object.freeze({
      async release() {
        if (released) return false;
        released = true;
        // Resolving the hold promise lets navigator.locks release the lease.
        release();
        await request;
        return true;
      }
    });
  }

  async function openDatabase(options, environment) {
    const sqlite3 = await sqlite();
    const storage = keywordName(option(options, "storage", "memory"));
    let database;
    let filename = ":memory:";
    let path = null;
    let lease = null;
    if (storage === "memory" || storage === "transient") {
      database = new sqlite3.oo1.DB(":memory:", "ct");
    } else if (storage === "filesystem") {
      if (environment !== "node" || !providerOptions.fileSystem) {
        throw new Error("db/sqlite-filesystem-unavailable: Node filesystem adapter is required");
      }
      const requested = option(options, "path");
      if (!requested) throw new Error("db/sqlite-path-required: filesystem storage requires :path");
      path = providerOptions.fileSystem.resolve(requested);
      filename = path;
      database = deserialize(sqlite3, await providerOptions.fileSystem.read(path));
    } else if (storage === "opfs") {
      if (environment !== "browser") {
        throw new Error("db/sqlite-opfs-unavailable: OPFS storage requires a browser worker");
      }
      path = String(option(options, "path", ""));
      if (!path.startsWith("/")) {
        throw new Error("db/sqlite-path-required: OPFS storage requires an absolute :path");
      }
      lease = await acquireOpfsLease(path);
      try {
        const pool = await opfsPool(sqlite3);
        database = new pool.OpfsSAHPoolDb(path);
        filename = path;
      } catch (error) {
        await lease.release();
        throw error;
      }
    } else {
      throw new Error(`db/sqlite-storage-unsupported: ${storage}`);
    }
    const id = ++nextConnectionId;
    connections.set(id, {
      id,
      sqlite3,
      database,
      environment,
      storage: storage === "transient" ? "memory" : storage,
      path,
      lease,
      tail: Promise.resolve()
    });
    return {
      id,
      engine: "sqlite",
      storage: storage === "transient" ? "memory" : storage,
      filename
    };
  }

  function serial(value, operation) {
    const pending = value.tail.then(operation, operation);
    value.tail = pending.then(() => undefined, () => undefined);
    return pending;
  }

  async function persist(value, before) {
    if (value.storage !== "filesystem") return;
    const bytes = exportDatabase(value);
    try {
      await providerOptions.fileSystem.writeAtomic(value.path, bytes);
    } catch (error) {
      try {
        value.database.close();
        value.database = deserialize(value.sqlite3, before);
      } catch (_) {
        connections.delete(value.id);
      }
      throw new Error("db/sqlite-persist: filesystem write failed");
    }
  }

  async function mutate(value, operation) {
    const before = value.storage === "filesystem" ? exportDatabase(value) : null;
    const result = await operation();
    await persist(value, before);
    return result;
  }

  function execDatabase(id, sql, params) {
    const value = connection(id);
    return serial(value, () => mutate(value, () => {
      const database = value.database;
      const bind = fromHta(params ?? []);
      const options = { sql: String(sql) };
      if (Array.isArray(bind) ? bind.length > 0 : bind != null) options.bind = bind;
      database.exec(options);
      return { affected: database.changes() };
    }));
  }

  function queryDatabase(id, sql, params) {
    const value = connection(id);
    return serial(value, () => {
      const database = value.database;
      const columns = [];
      const rows = [];
      const bind = fromHta(params ?? []);
      const options = {
        sql: String(sql),
        rowMode: "array",
        columnNames: columns,
        resultRows: rows
      };
      if (Array.isArray(bind) ? bind.length > 0 : bind != null) options.bind = bind;
      database.exec(options);
      return {
        columns,
        rows,
        affected: database.changes()
      };
    });
  }

  function callWorkStore(id, operation, args) {
    const value = connection(id);
    return serial(value, () => {
      const invoke = () => workCall(value.database, operation, args);
      return mutatesWorkStore(operation) ? mutate(value, invoke) : invoke();
    });
  }

  async function closeDatabase(id) {
    const key = Number(id);
    const value = connections.get(key);
    if (!value) return false;
    connections.delete(key);
    return serial(value, async () => {
      let failure = null;
      if (value.storage === "filesystem") {
        try {
          await providerOptions.fileSystem.writeAtomic(value.path, exportDatabase(value));
        } catch (_) {
          failure = new Error("db/sqlite-persist: filesystem write failed");
        }
      }
      try {
        try {
          value.database.close();
        } catch (_) {
          failure ??= new Error("db/sqlite-close: database close failed");
        }
      } finally {
        try {
          await value.lease?.release();
        } catch (_) {
          failure ??= new Error("db/sqlite-close: resource release failed");
        }
      }
      if (failure) throw failure;
      return true;
    });
  }

  async function call(environment, operation, args) {
    switch (operation) {
      case "version":
        return { engine: "sqlite", version: (await sqlite()).version.libVersion };
      case "open":
        return openDatabase(args[0], environment);
      case "exec":
        return execDatabase(args[0], args[1], args[2]);
      case "query":
        return queryDatabase(args[0], args[1], args[2]);
      case "work-call":
        return callWorkStore(args[0], args[1], args[2]);
      case "close":
        return closeDatabase(args[0]);
      default:
        throw new Error(`db/sqlite-operation-unknown: ${operation}`);
    }
  }

  async function closeAll() {
    await Promise.all(Array.from(connections.keys(), id => closeDatabase(id)));
  }

  return Object.freeze({ call, closeAll });
}
