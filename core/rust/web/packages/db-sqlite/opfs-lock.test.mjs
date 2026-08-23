import assert from "node:assert/strict";
import test from "node:test";
import { createSqliteProvider } from "./index.mjs";

class FakeDatabase {
  constructor(path) {
    this.path = path;
    this.pointer = {};
  }

  close() {
    this.closed = true;
  }
}

function sqliteModule() {
  return {
    installOpfsSAHPoolVfs: async () => ({ OpfsSAHPoolDb: FakeDatabase }),
    oo1: { DB: FakeDatabase },
    version: { libVersion: "test" },
    capi: {}
  };
}

function filesystemSqliteModule(databases) {
  class Database extends FakeDatabase {
    constructor(...args) {
      super(...args);
      databases.push(this);
    }
  }
  return {
    oo1: { DB: Database },
    version: { libVersion: "test" },
    capi: {
      sqlite3_js_db_export: () => new Uint8Array([1])
    }
  };
}

function options() {
  return new Map([
    ["storage", { name: "opfs" }],
    ["path", "/lease.db"]
  ]);
}

test("OPFS connections require an exclusive Web Lock and release it on close", async () => {
  const previous = globalThis.navigator.locks;
  let active = 0;
  let peak = 0;
  const waiters = [];
  Object.defineProperty(globalThis.navigator, "locks", {
    configurable: true,
    value: {
      request(name, mode, callback) {
        return new Promise((resolve, reject) => {
          waiters.push({ callback, resolve, reject });
          if (active || waiters.length !== 1) return;
          pump();
        });
      }
    }
  });

  async function pump() {
    if (active || !waiters.length) return;
    const next = waiters.shift();
    active = 1;
    peak = Math.max(peak, active);
    try {
      await next.callback({ name: "hara/sqlite/opfs:/lease.db" });
      next.resolve();
    } catch (error) {
      next.reject(error);
    } finally {
      active = 0;
      pump();
    }
  }

  try {
    const provider = createSqliteProvider(async () => sqliteModule());
    const first = await provider.call("browser", "open", [options()]);
    let secondOpened = false;
    const second = provider.call("browser", "open", [options()]).then(value => {
      secondOpened = true;
      return value;
    });
    await new Promise(resolve => setTimeout(resolve, 5));
    assert.equal(secondOpened, false);
    assert.equal(await provider.call("browser", "close", [first.id]), true);
    const secondConnection = await second;
    assert.equal(secondOpened, true);
    assert.equal(await provider.call("browser", "close", [secondConnection.id]), true);
    assert.equal(await provider.call("browser", "close", [secondConnection.id]), false);
    assert.equal(peak, 1);

    Object.defineProperty(globalThis.navigator, "locks", {
      configurable: true,
      value: undefined
    });
    await assert.rejects(
      () => provider.call("browser", "open", [options()]),
      /Web Locks are required/
    );
  } finally {
    Object.defineProperty(globalThis.navigator, "locks", {
      configurable: true,
      value: previous
    });
  }
});

test("filesystem close cleans up even when final persistence fails", async () => {
  const databases = [];
  const provider = createSqliteProvider(
    async () => filesystemSqliteModule(databases),
    {
      fileSystem: {
        resolve: path => `/tmp/${path}`,
        read: async () => null,
        writeAtomic: async () => {
          throw new Error("disk failure");
        }
      }
    }
  );
  const connection = await provider.call(
    "node",
    "open",
    [new Map([["storage", { name: "filesystem" }], ["path", "state.db"]])]
  );

  await assert.rejects(
    () => provider.call("node", "close", [connection.id]),
    { message: "db/sqlite-persist: filesystem write failed" }
  );
  assert.equal(databases.length, 1);
  assert.equal(databases[0].closed, true);
  assert.equal(await provider.call("node", "close", [connection.id]), false);
});
