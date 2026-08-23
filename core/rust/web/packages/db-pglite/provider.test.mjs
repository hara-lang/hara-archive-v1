import assert from "node:assert/strict";
import test from "node:test";
import { PGlite } from "@electric-sql/pglite";
import { createPgliteProvider } from "./index.mjs";

test("PGlite executes parameterized PostgreSQL through the db provider core", async () => {
  const pglite = createPgliteProvider(PGlite);
  const opened = await pglite.call("node", "open", [new Map()]);
  assert.equal(opened.engine, "postgresql");
  assert.equal(opened.provider, "pglite");
  assert.equal(opened.storage, "memory");
  assert.equal(opened.mode, "embedded");
  assert.ok(opened.capabilities.includes("notifications"));

  await pglite.call("node", "exec", [
    opened.id,
    "create table items (id serial primary key, name text not null)",
    []
  ]);
  const inserted = await pglite.call("node", "exec", [
    opened.id,
    "insert into items (name) values ($1)",
    ["wombat"]
  ]);
  assert.equal(inserted.affected, 1);

  const result = await pglite.call("node", "query", [
    opened.id,
    "select id, name from items where name = $1",
    ["wombat"]
  ]);
  assert.deepEqual(result.columns, ["id", "name"]);
  assert.deepEqual(result.rows, [[1, "wombat"]]);

  await pglite.call("node", "exec", [opened.id, "begin", []]);
  await pglite.call("node", "exec", [
    opened.id,
    "insert into items (name) values ($1)",
    ["rolled back"]
  ]);
  await pglite.call("node", "exec", [opened.id, "rollback", []]);
  const afterRollback = await pglite.call("node", "query", [
    opened.id,
    "select name from items order by id",
    []
  ]);
  assert.deepEqual(afterRollback.rows, [["wombat"]]);

  const simpleTypes = await pglite.call("node", "query", [
    opened.id,
    `select 12.3400::numeric,
            array[1, 2, null]::int8[],
            '[0:2]={4,5,6}'::int8[],
            array[[1,2],[3,4]]::int4[],
            jsonb_build_object('amount', 12.34, 'items', array[1,2])`,
    []
  ]);
  assert.equal(simpleTypes.rows[0][0], 12.34);
  assert.deepEqual(simpleTypes.rows[0][1], [1, 2, null]);
  assert.deepEqual(simpleTypes.rows[0][2], {
    $postgres: "array",
    element: "int8",
    dimensions: [[0, 3]],
    value: [4, 5, 6]
  });
  assert.deepEqual(simpleTypes.rows[0][3], [[1, 2], [3, 4]]);
  assert.deepEqual(simpleTypes.rows[0][4], { amount: 12.34, items: [1, 2] });

  const taggedTypes = await pglite.call("node", "query", [
    opened.id,
    "select 12.3400::numeric, array[1,2]::numeric[], array[]::int8[]",
    [],
    new Map([["decode", { name: "tagged" }]])
  ]);
  assert.deepEqual(taggedTypes.rows[0], [
    { $postgres: "numeric", value: "12.3400" },
    {
      $postgres: "array",
      element: "numeric",
      dimensions: [[1, 2]],
      value: [
        { $postgres: "numeric", value: "1" },
        { $postgres: "numeric", value: "2" }
      ]
    },
    { $postgres: "array", element: "int8", dimensions: [], value: [] }
  ]);

  const numericParameter = await pglite.call("node", "query", [
    opened.id,
    "select $1",
    [new Map([["$postgres", "numeric"], ["value", "9007199254740993.25"]])],
    new Map([["decode", "tagged"]])
  ]);
  assert.deepEqual(numericParameter.rows[0][0], {
    $postgres: "numeric",
    value: "9007199254740993.25"
  });

  const arrayParameter = await pglite.call("node", "query", [
    opened.id,
    "select $1",
    [new Map([
      ["$postgres", "array"], ["element", "int8"],
      ["dimensions", [[0, 3]]], ["value", [7, 8, 9]]
    ])],
    new Map([["decode", "tagged"]])
  ]);
  assert.deepEqual(arrayParameter.rows[0][0], {
    $postgres: "array",
    element: "int8",
    dimensions: [[0, 3]],
    value: [7, 8, 9]
  });

  const literalJsonTag = { $postgres: "numeric", value: "1.25" };
  const jsonParameter = await pglite.call("node", "query", [
    opened.id,
    "select $1::jsonb",
    [new Map(Object.entries(literalJsonTag))]
  ]);
  assert.deepEqual(jsonParameter.rows[0][0], literalJsonTag);

  const subscription = await pglite.call("node", "listen", [opened.id, "items_changed"]);
  const pending = pglite.call("node", "notification-next", [subscription.id]);
  await pglite.call("node", "notify", [opened.id, "items_changed", "wombat"]);
  assert.deepEqual(await pending, { channel: "items_changed", payload: "wombat", pid: null });
  assert.equal(await pglite.call("node", "unlisten", [subscription.id]), true);

  await assert.rejects(
    pglite.call("node", "database-create", [new Map(), "another"]),
    /postgres\/capability-unsupported/
  );

  const closedSubscription = await pglite.call("node", "listen", [opened.id, "close_check"]);
  assert.equal(await pglite.call("node", "close", [opened.id]), true);
  await assert.rejects(
    pglite.call("node", "notification-next", [closedSubscription.id]),
    /postgres\/subscription-closed/
  );
  assert.equal(await pglite.call("node", "close", [opened.id]), false);
  await assert.rejects(
    pglite.call("node", "query", [opened.id, "select 1", []]),
    /db\/pglite-connection-missing/
  );
});
