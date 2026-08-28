import { test, expect } from "@playwright/test";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

test("Chromium compiles and executes a Hara whole-Wasm function", async ({ page }) => {
  await page.goto("/rust/web/index.html");
  const result = await page.evaluate(async () => {
    const { start } = await import("/rust/web/packages/browser/dist/hara-wasm-full/hara.mjs");
    const hara = await start();
    const compiled = await hara.compileWholeWasm(
      "(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc i)) acc))"
    );
    return String(compiled.call());
  });
  expect(result).toBe("12497500");
});

test("Chromium executes the exact HNW0 artifact already run by Wasmtime", async ({ page }) => {
  await page.goto("/rust/web/index.html");
  const observed = await page.evaluate(async () => {
    const { start } = await import("/rust/web/packages/browser/dist/hara-wasm-full/hara.mjs");
    const { decodeHnw0 } = await import("/rust/web/packages/browser/src/whole-wasm.js");
    const response = await fetch("/target/whole-wasm-native-browser-parity.hnw");
    if (!response.ok) {
      throw new Error(`unable to fetch parity artifact: ${response.status}`);
    }
    const artifact = new Uint8Array(await response.arrayBuffer());
    const decoded = decodeHnw0(artifact);
    const imports = [...WebAssembly.Module.imports(new WebAssembly.Module(decoded.wasm))]
      .map(({ module, name }) => `${module}::${name}`);
    const unsupported = artifact.slice();
    unsupported[8] = 0;
    unsupported[9] = 1;
    let rejection;
    try {
      decodeHnw0(unsupported);
    } catch (error) {
      rejection = error.message;
    }
    const hara = await start();
    const compiled = await hara.loadWholeWasm(artifact);
    return {
      magic: String.fromCharCode(...artifact.subarray(0, 4)),
      byteLength: artifact.byteLength,
      abiVersion: decoded.abiVersion,
      targets: decoded.targets.map(({ symbol, arity }) => ({ symbol, arity })),
      imports,
      rejection,
      results: Array.from({ length: 8 }, () => String(compiled.call()))
    };
  });

  expect(observed.magic).toBe("HNW0");
  expect(observed.byteLength).toBeGreaterThan(40);
  expect(observed.abiVersion).toBe(0);
  expect(observed.targets).toEqual([
    { symbol: "hara.whole-wasm/map", arity: null },
    { symbol: "hara.whole-wasm/vector", arity: null },
    { symbol: "std.native.Base/number?", arity: 1 },
    { symbol: "std.protocol.iassoc.IAssoc/assoc", arity: 3 },
    { symbol: "std.protocol.icount.ICount/count", arity: 1 },
    { symbol: "std.protocol.ilookup.ILookup/lookup", arity: 2 },
    { symbol: "std.protocol.inth.INth/nth", arity: 2 }
  ]);
  expect(observed.imports).toEqual([
    "hara::constant_handle",
    "hara::box_i64",
    "hara::unbox_i64",
    "hara::value_construct",
    "hara::target_call"
  ]);
  expect(observed.rejection).toMatch(/unsupported HNW ABI version 1/);
  expect(observed.results).toEqual(Array(8).fill("12497500"));
});

test("Chromium records five-workload whole-Wasm parity and timing evidence", async ({ page }) => {
  test.setTimeout(120_000);
  await page.goto("/rust/web/index.html");
  const report = await page.evaluate(async () => {
    const corpus = await fetch("/rust/assets/whole-wasm-workloads.json").then((response) => response.json());
    const { start } = await import("/rust/web/packages/browser/dist/hara-wasm-full/hara.mjs");
    const hara = await start();
    const measurements = [];
    for (const workload of corpus.workloads) {
      const vmArtifact = hara.compileBytecode(workload.hara_source);
      const vmCall = () => String(hara.evalBytecode(vmArtifact));
      if (vmCall() !== workload.expected) {
        throw new Error(`${workload.id}: browser HBC checksum mismatch`);
      }
      for (let index = 0; index < 2; index += 1) vmCall();
      const vmStarted = performance.now();
      for (let index = 0; index < 3; index += 1) vmCall();
      const vmSteadyNs = Math.round((performance.now() - vmStarted) * 1e6 / 3);

      const prepareStarted = performance.now();
      const compiled = await hara.compileWholeWasm(workload.hara_source);
      const prepareNs = Math.round((performance.now() - prepareStarted) * 1e6);
      const firstStarted = performance.now();
      if (String(compiled.call()) !== workload.expected) {
        throw new Error(`${workload.id}: browser whole-Wasm checksum mismatch`);
      }
      const firstNs = Math.round((performance.now() - firstStarted) * 1e6);
      for (let index = 0; index < 2; index += 1) compiled.call();
      const steadyStarted = performance.now();
      for (let index = 0; index < 3; index += 1) compiled.call();
      const steadyNs = Math.round((performance.now() - steadyStarted) * 1e6 / 3);
      measurements.push({
        id: workload.id,
        expected: workload.expected,
        native: compiled.host.supportsNative(0n),
        prepare_ns: prepareNs,
        first_ns: firstNs,
        vm_steady_ns: vmSteadyNs,
        whole_wasm_steady_ns: steadyNs,
        status: "ok"
      });
    }
    return { schema: "hara.whole-wasm.browser-performance/0-alpha", measurements };
  });
  expect(report.measurements).toHaveLength(5);
  expect(report.measurements.every((measurement) => measurement.status === "ok")).toBe(true);
  expect(report.measurements.every((measurement) => measurement.native)).toBe(true);
  const output = resolve(import.meta.dirname, "../../target/whole-wasm-browser-performance.json");
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);
});

test("Chromium/Wasm produces deterministic instrumentation conformance evidence", async ({ page }) => {
  await page.goto("/rust/web/index.html");
  const reports = await page.evaluate(async () => {
    const corpus = await fetch("/spec/instrumentation-conformance.json").then((response) => response.json());
    const { start } = await import("/rust/web/packages/browser/dist/hara-wasm-full/hara.mjs");
    const hara = await start();
    return [hara.instrumentationConformance(corpus), hara.instrumentationConformance(corpus)];
  });
  expect(reports[0]).toEqual(reports[1]);
  expect(reports[0].runtime).toBe("wasm");
  expect(reports[0].schema).toBe("hara.instrumentation.conformance-report/1");
  expect(reports[0].cases).toHaveLength(11);
  expect(reports[0].cases[0].observation.eventOrder).toEqual([
    "semantic-boundary",
    "execution-terminal"
  ]);
  expect(reports[0].cases[9].reset.generation).toBe(1);
  expect(reports[0].cases[10].result).toBe("3");
});
