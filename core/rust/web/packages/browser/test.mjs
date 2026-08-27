import assert from "node:assert/strict";
import test from "node:test";
import { start as startVm } from "./dist/hara-wasm-vm/hara.mjs";
import { start as startFull } from "./dist/hara-wasm-full/hara.mjs";

const MEMORY_MODULE = Uint8Array.from([
  0, 97, 115, 109, 1, 0, 0, 0, 1, 20, 4, 96, 1, 127, 1, 127, 96, 1, 127, 0,
  96, 2, 127, 127, 1, 126, 96, 0, 1, 127, 3, 5, 4, 0, 1, 2, 3, 5, 4, 1, 1,
  1, 16, 6, 6, 1, 127, 1, 65, 0, 11, 7, 54, 5, 6, 109, 101, 109, 111, 114,
  121, 2, 0, 5, 97, 108, 108, 111, 99, 0, 0, 4, 102, 114, 101, 101, 0, 1,
  10, 101, 99, 104, 111, 95, 98, 121, 116, 101, 115, 0, 2, 13, 114, 101,
  108, 101, 97, 115, 101, 95, 99, 111, 117, 110, 116, 0, 3, 10, 35, 4, 5, 0,
  65, 128, 8, 11, 9, 0, 35, 0, 65, 1, 106, 36, 0, 11, 12, 0, 32, 1, 173, 66,
  32, 134, 32, 0, 173, 132, 11, 4, 0, 35, 0, 11
]);

const MEMORY_INTERFACE = `(wasm/interface
  {:schema "hara.wasm-interface/0-alpha"
   :namespace codec.bytes
   :module "echo.wasm"
   :memory {:export "memory" :allocate "alloc" :release "free"}
   :exports
   {echo {:wasm/export "echo_bytes"
          :arguments [{:name input
                       :hara/type :bytes
                       :wasm/type :i32
                       :lower [:pointer :length]
                       :ownership :borrowed}]
          :returns {:hara/type :bytes
                    :wasm/type :i64
                    :lift :packed-i64
                    :ownership :caller}}
    release-count {:wasm/export "release_count"
                   :arguments []
                   :returns {:hara/type :i32 :wasm/type :i32}}}})`;

const MEMORY_BINDINGS =
  `{:schema "hara.wasm-memory-binding/0-alpha" :namespace codec.bytes :module "echo.wasm" :target :memory.v1 :memory {:export "memory" :allocate "alloc" :release "free"} :functions [{:hara/name echo :wasm/export "echo_bytes" :arguments [{:name input :hara/type :bytes :wasm/types [:i32 :i32] :lower [:pointer :length] :ownership :borrowed}] :returns {:hara/type :bytes :wasm/type :i64 :lift :packed-i64 :ownership :caller} :wasm/arguments [:i32 :i32] :wasm/returns :i64} {:hara/name release-count :wasm/export "release_count" :arguments [] :returns {:hara/type :i32 :wasm/type :i32} :wasm/arguments [] :wasm/returns :i32}]}`;

const MEMORY_MANIFEST = `{:namespace "codec.bytes" :version "0.1.0" :provider :wasm :module "echo.wasm" :abi :memory.v1 :exports {"echo" {:wasm/export "echo_bytes" :args [:bytes] :returns :bytes :async false} "release-count" {:wasm/export "release_count" :args [] :returns :i32 :async false}} :capabilities [] :assets ["bindings.edn"]}`;

const MEMORY_STRING_INTERFACE = MEMORY_INTERFACE
  .replaceAll("codec.bytes", "codec.string")
  .replaceAll(":bytes", ":string");
const MEMORY_STRING_BINDINGS = MEMORY_BINDINGS
  .replaceAll("codec.bytes", "codec.string")
  .replaceAll(":bytes", ":string");
const MEMORY_STRING_MANIFEST = MEMORY_MANIFEST.replaceAll(
  "codec.bytes",
  "codec.string"
).replaceAll(":bytes", ":string");

test("browser SDK starts the embedded runtime and loads supplied resources", async () => {
  const hara = await startVm({
    resources: {
      "app.config": "(ns app.config) (def answer 42) 42"
    }
  });

  assert.equal(hara.require("app.config"), "42");
  assert.equal(hara.eval("app.config/answer"), "42");
});

test("browser SDK starts isolated core runtimes", async () => {
  const first = await startVm({
    resources: { "app.first": "(ns app.first) (def answer 41)" }
  });
  const second = await startVm();
  try {
    assert.notEqual(first.raw, second.raw);
    first.require("app.first");
    assert.equal(first.eval("app.first/answer"), "41");
    assert.throws(() => second.eval("app.first/answer"), /unbound symbol/);
  } finally {
    await first.dispose();
    await second.dispose();
  }
});

test("browser SDK executes the generic memory.v1 bytes binding", async () => {
  const hara = await startVm({
    resources: {
      "std.foundation":
        "(ns std.foundation) "
        + "(defn bytes [a b c d] (std.native.Bytes/new a b c d))"
    }
  });
  hara.require("std.foundation");
  hara.installMemoryWasmBinding(
    MEMORY_MANIFEST,
    MEMORY_INTERFACE,
    MEMORY_BINDINGS,
    MEMORY_MODULE
  );
  assert.equal(hara.require("codec.bytes"), ":loaded");
  assert.equal(
    hara.eval(
      "(ns browser.memory (:require [codec.bytes :as cb])) " +
        "(cb/echo (bytes 1 2 3 4))"
    ),
    "#bytes[1 2 3 4]"
  );
  assert.equal(hara.eval("(codec.bytes/release-count)"), "1");
  hara.installMemoryWasmBinding(
    MEMORY_STRING_MANIFEST,
    MEMORY_STRING_INTERFACE,
    MEMORY_STRING_BINDINGS,
    MEMORY_MODULE
  );
  assert.equal(hara.require("codec.string"), ":loaded");
  assert.equal(
    hara.eval(
      "(ns browser.memory.string (:require [codec.string :as cs])) " +
        '(cs/echo "hara memory binding")'
    ),
    '"hara memory binding"'
  );
  assert.equal(hara.eval("(codec.string/release-count)"), "1");
});

test("browser SDK validates memory.v1 metadata before installation", async () => {
  const hara = await startVm();
  assert.throws(
    () =>
      hara.installMemoryWasmBinding(
        MEMORY_MANIFEST.replace("codec.bytes", "codec.invalid"),
        MEMORY_INTERFACE.replace("codec.bytes", "codec.invalid"),
        `${MEMORY_BINDINGS} `,
        MEMORY_MODULE
      ),
    /canonical memory\.v1 plan/
  );
  assert.throws(
    () =>
      hara.installMemoryWasmBinding(
        MEMORY_MANIFEST.replace("codec.bytes", "codec.invalid"),
        MEMORY_INTERFACE,
        MEMORY_BINDINGS,
        MEMORY_MODULE
      ),
    /manifest namespace or module/
  );
});

test("VM package compiles and executes persistent bytecode", async () => {
  const hara = await startVm();
  const artifact = hara.compileBytecode("(+ 19 23)");
  assert.equal(hara.evalBytecode(artifact), "42");
  await assert.rejects(() => hara.compileWholeWasm("(+ 19 23)"), /browser\/full/);
});

test("browser runtime binds typed direct-WASM imports without HTA", async () => {
  const hara = await startVm();
  const add = Uint8Array.from([
    0, 97, 115, 109, 1, 0, 0, 0, 1, 7, 1, 96, 2, 126, 126, 1, 126,
    3, 2, 1, 0, 7, 7, 1, 3, 97, 100, 100, 0, 0, 10, 9, 1, 7, 0,
    32, 0, 32, 1, 124, 11
  ]);
  hara.installDirectWasmImport("math", add);
  assert.equal(
    hara.eval("(ns browser.direct (:import math)) (math/add 20 22)"),
    "42"
  );
});

test("browser SDK compiles and executes whole-function WebAssembly", async () => {
  const hara = await startFull();
  const compiled = await hara.compileWholeWasm(
    "(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc i)) acc))"
  );
  assert.equal(compiled.call(), 12_497_500n);

  const product = hara.compileWholeWasmProduct("(+ 19 23)");
  assert.equal(product.manifest["abi-version"], "hnw0/0");
  const loaded = await hara.loadWholeWasm(product);
  assert.equal(loaded.call(), 42n);
  await assert.rejects(
    () => hara.loadWholeWasm({
      artifact: product.artifact,
      manifest: { ...product.manifest, "artifact-digest": "00".repeat(32) }
    }),
    /manifest digest does not match artifact/
  );

  const division = await hara.compileWholeWasm("(/ 1 0)");
  assert.throws(() => division.call(), /division by zero/);
  hara.dispose();
});
