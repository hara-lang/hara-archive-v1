import { test } from "@playwright/test";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const corpusUrl = process.env.HARA_INTEGER_BENCHMARK_CORPUS_URL;
const standard = process.env.HARA_BENCH_PROFILE === "standard";
test.skip(!corpusUrl, "set HARA_INTEGER_BENCHMARK_CORPUS_URL to run issue #1145 evidence");
test.setTimeout(standard ? 45 * 60_000 : 10 * 60_000);

test("records paired integer representation evidence in Chromium", async ({ page }) => {
  await page.goto("/rust/web/index.html");
  const report = await page.evaluate(async ({ corpusUrl, standard }) => {
    const corpus = await fetch(corpusUrl).then((response) => {
      if (!response.ok) throw new Error(`integer benchmark corpus: ${response.status}`);
      return response.json();
    });
    const { start } = await import("/rust/web/packages/browser/dist/hara-wasm-full/hara.mjs");
    const hara = await start();
    const median = (values) => [...values].sort((left, right) => left - right)[Math.floor(values.length / 2)];
    const profile = {
      ...(corpus.profiles?.[standard ? "standard" : "smoke"] ??
        (standard
          ? { windows: 60, calls: 10 }
          : { windows: 3, calls: 1 })),
      targetMilliseconds: standard ? 250 : 25,
    };
    const measurements = [];
    const expanded = [];
    for (const workload of corpus.workloads) {
      for (const [candidate, definition] of Object.entries(workload.candidates)) {
        expanded.push({
          ...workload,
          ...definition,
          id: `${workload.id}/${candidate}`,
          pair_id: workload.id,
          candidate,
          expected: typeof workload.expected === "object"
            ? workload.expected[candidate]
            : workload.expected,
        });
      }
    }
    const measure = (call) => {
      for (let index = 0; index < 5; index += 1) call();
      let calls = profile.calls;
      while (calls < 16_777_216) {
        const started = performance.now();
        for (let index = 0; index < calls; index += 1) call();
        const elapsed = performance.now() - started;
        if (elapsed >= profile.targetMilliseconds) break;
        calls *= Math.max(2, Math.ceil(profile.targetMilliseconds / Math.max(elapsed, 0.01)));
      }
      const samples = [];
      for (let window = 0; window < profile.windows; window += 1) {
        const started = performance.now();
        for (let index = 0; index < calls; index += 1) call();
        samples.push(Math.round((performance.now() - started) * 1e6 / calls));
      }
      const steadyNs = median(samples);
      return {
        steady_ns: steadyNs,
        samples_ns: samples,
        calls_per_window: calls,
        throughput_per_sec: 1e9 / steadyNs,
      };
    };
    const wholeDisplay = (value, expected) => {
      if (expected === "true" && (value === 1n || value === 1)) return "true";
      if (expected === "false" && (value === 0n || value === 0)) return "false";
      return String(value);
    };
    try {
      for (const workload of expanded) {
        const sourceBytes = new TextEncoder().encode(workload.source).byteLength;
        const vmPrepareStarted = performance.now();
        const vmArtifact = hara.compileBytecode(workload.source);
        const vmPrepareNs = Math.round((performance.now() - vmPrepareStarted) * 1e6);
        const vmCall = () => String(hara.evalBytecode(vmArtifact));
        const vmFirstStarted = performance.now();
        const vmFirst = vmCall();
        const vmFirstNs = Math.round((performance.now() - vmFirstStarted) * 1e6);
        if (vmFirst !== workload.expected) throw new Error(`${workload.id}: VM checksum ${vmFirst}`);
        measurements.push({
          runtime: "hara-browser-vm",
          workload: workload.id,
          pair_id: workload.pair_id,
          candidate: workload.candidate,
          lane: workload.lane,
          operation: workload.operation,
          value_class: workload.value_class,
          representation: workload.representation,
          result_kind: workload.result_kind,
          prepare_ns: vmPrepareNs,
          first_ns: vmFirstNs,
          source_bytes: sourceBytes,
          artifact_bytes: vmArtifact.byteLength,
          allocation_bytes_per_call: null,
          allocation_unsupported_reason: "browser allocation counters are unavailable",
          peak_rss_bytes: null,
          execution_path: "bytecode",
          ...measure(vmCall),
          checksum: workload.expected,
          status: "ok",
        });

        let wholeProduct = null;
        let compiled = null;
        let wholePrepareNs = null;
        let nativeEntry = null;
        try {
          const wholePrepareStarted = performance.now();
          wholeProduct = hara.compileWholeWasmProduct(workload.source);
          compiled = await hara.loadWholeWasm(wholeProduct);
          wholePrepareNs = Math.round((performance.now() - wholePrepareStarted) * 1e6);
          nativeEntry = compiled.host.supportsNative(BigInt(compiled.entryFunction()));
          const wholeCall = () => wholeDisplay(compiled.call(), workload.expected);
          const wholeFirstStarted = performance.now();
          const wholeFirst = wholeCall();
          const wholeFirstNs = Math.round((performance.now() - wholeFirstStarted) * 1e6);
          if (wholeFirst !== workload.expected) {
            throw new Error(`${workload.id}: whole-Wasm checksum mismatch (expected ${workload.expected}, got ${wholeFirst})`);
          }
          measurements.push({
            runtime: "hara-browser-whole-wasm",
            workload: workload.id,
            pair_id: workload.pair_id,
            candidate: workload.candidate,
            lane: workload.lane,
            operation: workload.operation,
            value_class: workload.value_class,
            representation: workload.representation,
            result_kind: workload.result_kind,
            prepare_ns: wholePrepareNs,
            first_ns: wholeFirstNs,
            source_bytes: sourceBytes,
            artifact_bytes: wholeProduct.artifact.byteLength,
            allocation_bytes_per_call: null,
            allocation_unsupported_reason: "browser allocation counters are unavailable",
            peak_rss_bytes: null,
            native_entry: nativeEntry,
            execution_path: nativeEntry ? "native" : "fallback",
            ...measure(wholeCall),
            checksum: workload.expected,
            status: "ok",
          });
        } catch (error) {
          const reason = error instanceof Error ? error.message : String(error);
          const status = reason.includes("checksum mismatch") ? "error" : "unsupported";
          measurements.push({
            runtime: "hara-browser-whole-wasm",
            workload: workload.id,
            pair_id: workload.pair_id,
            candidate: workload.candidate,
            lane: workload.lane,
            operation: workload.operation,
            value_class: workload.value_class,
            representation: workload.representation,
            result_kind: workload.result_kind,
            prepare_ns: wholePrepareNs,
            first_ns: null,
            source_bytes: sourceBytes,
            artifact_bytes: wholeProduct?.artifact?.byteLength ?? null,
            allocation_bytes_per_call: null,
            allocation_unsupported_reason: "browser allocation counters are unavailable",
            peak_rss_bytes: null,
            native_entry: nativeEntry,
            execution_path: nativeEntry == null ? null : nativeEntry ? "native" : "fallback",
            checksum: workload.expected,
            status,
            reason,
          });
        }
      }
    } finally {
      await hara.dispose();
    }
    const errors = measurements.filter((row) => row.status === "error");
    if (errors.length) throw new Error(errors.map((row) => row.reason).join("; "));
    const groups = new Map();
    for (const row of measurements) {
      const key = `${row.runtime}|${row.pair_id}|${row.lane}`;
      const group = groups.get(key) ?? {};
      group[row.candidate] = row;
      groups.set(key, group);
    }
    const candidateComparisons = [];
    for (const [key, group] of groups) {
      if (!group.split || !group.unified ||
          typeof group.split.steady_ns !== "number" ||
          typeof group.unified.steady_ns !== "number") continue;
      const [runtime, pairId, lane] = key.split("|");
      candidateComparisons.push({
        runtime,
        pair_id: pairId,
        lane,
        split_steady_ns: group.split.steady_ns,
        unified_steady_ns: group.unified.steady_ns,
        unified_over_split: group.unified.steady_ns / group.split.steady_ns,
        faster_candidate: group.split.steady_ns < group.unified.steady_ns
          ? "split"
          : group.unified.steady_ns < group.split.steady_ns ? "unified" : "tie",
      });
    }
    return {
      schema_version: 1,
      benchmark: corpus.benchmark,
      issue: corpus.issue,
      decision_policy: corpus.decision_policy,
      storage_model: corpus.storage_model,
      corpus_schema_version: corpus.schema_version,
      profile: standard ? "standard" : "smoke",
      candidate_comparisons: candidateComparisons,
      measurements,
    };
  }, { corpusUrl, standard });
  console.log(`INTEGER_REPRESENTATION_BROWSER_BENCHMARK=${JSON.stringify(report)}`);
  const output = resolve(import.meta.dirname, "../../target/integer-representation-browser.json");
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, `${JSON.stringify({
    ...report,
    generated_at: new Date().toISOString(),
  }, null, 2)}\n`);
});
