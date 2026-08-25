import { HtaContext, HtaKeyword, HTA_BROWSER_WORKER_URL } from "./packages/hta/index.js";

const kw = name => new HtaKeyword(name);
const map = entries => new Map(entries.map(([key, value]) => [kw(key), value]));
const value = (input, name) => {
  for (const [key, item] of input) if (key.name === name) return item;
  return undefined;
};

function provider() {
  const worker = new Worker(HTA_BROWSER_WORKER_URL, { type: "module" });
  const context = new HtaContext({
    worker,
    providerUrl: new URL("./dist-sqlite-browser/provider.mjs", import.meta.url).toString()
  });
  return {
    call: (operation, args) => context.call(operation, args),
    close: () => context.close()
  };
}

async function open(value, path) {
  return value.call("open", [map([["storage", kw("opfs")], ["path", path]])]);
}

async function work(client, connection, operation, ...args) {
  return client.call("work-call", [value(connection, "id"), kw(operation), args]);
}

async function opfsRestartConformance() {
  const path = `/hara-work-${crypto.randomUUID()}.db`;
  const first = provider();
  const firstConnection = await open(first, path);
  await work(first, firstConnection, "migrate");
  await work(first, firstConnection, "create-run", map([
    ["run/id", "browser-restart"],
    ["run/work-root", kw("test/browser-restart")],
    ["run/work-version", 1],
    ["run/input", 42]
  ]));
  await work(first, firstConnection, "transact", map([
    ["transition/run-id", "browser-restart"],
    ["transition/expected-revision", 0],
    ["transition/run-updates", map([["run/status", kw("running")]])],
    ["transition/outbox", [map([
      ["outbox/key", ["browser-restart", kw("receipt"), kw("final")]],
      ["outbox/topic", kw("work/receipt")],
      ["outbox/payload", map([["run/id", "browser-restart"]])]
    ])]]
  ]));
  await first.call("close", [value(firstConnection, "id")]);
  first.close();

  const second = provider();
  const secondConnection = await open(second, path);
  await work(second, secondConnection, "migrate");
  const loaded = await work(second, secondConnection, "load-run", "browser-restart");
  const claimed = await work(
    second,
    secondConnection,
    "claim-outbox",
    map([["claim/id", "browser-publisher"], ["limit", 1]])
  );
  await work(
    second,
    secondConnection,
    "ack-outbox",
    value(claimed[0], "outbox/id"),
    map([["claim/id", "browser-publisher"], ["ack/data", map([["published", true]])]])
  );
  await second.call("close", [value(secondConnection, "id")]);
  second.close();

  const third = provider();
  const thirdConnection = await open(third, path);
  await work(third, thirdConnection, "migrate");
  const redelivery = await work(
    third,
    thirdConnection,
    "claim-outbox",
    map([["claim/id", "replacement-publisher"], ["limit", 1]])
  );
  const acknowledged = await work(
    third,
    thirdConnection,
    "list-outbox",
    map([["status", kw("acked")]])
  );
  await third.call("close", [value(thirdConnection, "id")]);
  third.close();
  return {
    input: value(loaded, "run/input"),
    firstDelivery: claimed.length,
    redelivery: redelivery.length,
    acknowledged: acknowledged.length
  };
}

window.sqliteOpfsConformance = opfsRestartConformance();
