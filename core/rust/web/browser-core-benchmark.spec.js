import { test } from "@playwright/test";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const standard = process.env.HARA_BENCH_PROFILE === "standard";
test.setTimeout(standard ? 30 * 60_000 : 5 * 60_000);

test("benchmark the modular browser interpreter core", async ({ page }) => {
  await page.goto("/rust/web/index.html");
  const result = await page.evaluate(async ({ standard }) => {
    const corpus = await fetch("/lib/bench/lisp-hara/general-workloads.json").then((response) => response.json());
    const { HtaContext } = await import("/rust/web/packages/hta/index.js");
    const moduleBytes = await fetch("/rust/raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-core.wasm").then((response) => response.arrayBuffer());
    const workerUrl = "/rust/web/packages/hta/worker.mjs";
    const createContext = async () => {
      const context = new HtaContext({ worker: new Worker(workerUrl, { type: "module" }), moduleBytes });
      await context.ready;
      return context;
    };
    const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
    const startupSamples = [];
    for (let index = 0; index < (standard ? 30 : 2); index += 1) {
      const started = performance.now();
      const context = await createContext();
      startupSamples.push(Math.round((performance.now() - started) * 1e6));
      context.close();
    }
    const context = await createContext();
    const measurements = [];
    for (const workload of corpus.workloads) {
      const prepareStarted = performance.now();
      const session = await context.createSession(`core-${workload.id}`);
      const prepareNs = Math.round((performance.now() - prepareStarted) * 1e6);
      const call = async () => {
        const value = await session.eval(workload.hara_source);
        if (String(value) !== workload.expected) throw new Error(`core checksum: ${value}`);
      };
      let started = performance.now();
      await call();
      const firstNs = Math.round((performance.now() - started) * 1e6);
      // The core API intentionally exposes source evaluation rather than a
      // prepared bytecode handle. The first evaluation is therefore the
      // representative end-to-end sample; repeating recursive sources can
      // take tens of minutes and would not become prepared execution.
      const windowCount = 0;
      const calls = 1;
      const samples = [firstNs];
      for (let window = 0; window < windowCount; window += 1) {
        started = performance.now();
        for (let index = 0; index < calls; index += 1) await call();
        samples.push(Math.round((performance.now() - started) * 1e6 / calls));
      }
      const steadyNs = median(samples);
      measurements.push({ runtime: "hara-wasm-core", workload: workload.id,
        prepare_ns: prepareNs, first_ns: firstNs, steady_ns: steadyNs,
        throughput_per_sec: 1e9 / steadyNs, checksum: workload.expected,
        samples_ns: samples, calls_per_window: calls, status: "ok" });
      await session.close();
    }
    context.close();
    return { startup: { samples_ns: startupSamples, p50_ns: median(startupSamples) }, measurements };
  }, { standard });
  console.log(`BROWSER_CORE_BENCHMARK=${JSON.stringify(result)}`);
  const output = resolve(import.meta.dirname, "../../target/browser-core-benchmark.json");
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, `${JSON.stringify({ schema_version: 1,
    profile: standard ? "standard" : "smoke", generated_at: new Date().toISOString(), ...result }, null, 2)}\n`);
});
