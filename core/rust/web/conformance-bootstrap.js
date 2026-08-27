import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const foundationResources = [
  ["std.foundation", "../../lib/src/std/foundation.hal"],
  ["std.foundation.promise", "../../lib/src/std/foundation/promise.hal"],
  ["std.foundation.string", "../../lib/src/std/foundation/string.hal"],
  ["std.foundation.bytes", "../../lib/src/std/foundation/bytes.hal"],
  ["std.foundation.coroutine", "../../lib/src/std/foundation/coroutine.hal"],
  ["std.foundation.pretty", "../../lib/src/std/foundation/pretty.hal"]
];

export async function readFoundationResources() {
  return Object.fromEntries(
    await Promise.all(
      foundationResources.map(async ([namespace, relative]) => [
        namespace,
        await readFile(
          fileURLToPath(new URL(relative, import.meta.url)),
          "utf8"
        )
      ])
    )
  );
}
