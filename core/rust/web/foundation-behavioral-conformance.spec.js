import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { test, expect } from "@playwright/test";
import { readFoundationResources } from "./conformance-bootstrap.js";

const corpusUrl = new URL(
  "../../../../hara-specs-registry/01-lang/004-foundation/draft/conformance/fixtures/foundation_behavioral.hal",
  import.meta.url
);

test("browser Wasm consumes the specs-owned Foundation behavioral corpus", async ({ page }) => {
  const corpus = await readFile(fileURLToPath(corpusUrl), "utf8");
  const resources = await readFoundationResources();

  await page.goto("/rust/web/index.html");
  const observed = await page.evaluate(async ({ resources, corpus }) => {
    const { start } = await import(
      "/rust/web/packages/browser/dist/hara-wasm-full/hara.mjs"
    );
    const hara = await start({ resources });
    for (const namespace of Object.keys(resources)) hara.require(namespace);
    hara.eval("(ns user)");
    if (
      !hara.raw ||
      typeof hara.eval !== "function" ||
      typeof hara.compileBytecode !== "function" ||
      typeof hara.evalBytecode !== "function"
    ) {
      throw new Error("full browser runtime is absent");
    }

    hara.eval(corpus);
    const summary = String(hara.eval("(foundation-summary-report)"));
    const valid = String(hara.eval("(foundation-corpus-valid?)"));
    const portablePass = String(
      hara.eval("(every? :pass (:results (foundation-profile-report)))")
    );
    const boundaryPass = String(
      hara.eval("(every? :pass (foundation-boundary-results))")
    );
    const calibrationPass = String(
      hara.eval("(every? :pass (foundation-calibration-results))")
    );
    const derivedTotals = String(
      hara.eval(
        "(let [report (foundation-summary-report)] (and (= (:surface report) (:classified report)) (= (:surface report) (+ (:portable report) (:capability-specific report) (:inventory-only report))) (= (:portable report) (+ (:passed report) (:failed report))) (= (:skipped report) (+ (:capability-specific report) (:inventory-only report)))))"
      )
    );
    const probe = JSON.parse(
      String(
        hara.eval(
          "(get (get foundation-calibration-snippets :compact-vector-type-boundary) :source)"
        )
      )
    );
    const probeExpected = String(
      hara.eval(
        "(get (get foundation-calibration-snippets :compact-vector-type-boundary) :expected)"
      )
    );
    const interpreted = String(hara.eval(probe));
    const artifact = hara.compileBytecode(probe);
    const compiled = String(hara.evalBytecode(artifact));
    return {
      summary,
      valid,
      portablePass,
      boundaryPass,
      calibrationPass,
      derivedTotals,
      probeExpected,
      interpreted,
      compiled
    };
  }, { resources, corpus });

  expect(observed.valid).toBe("true");
  expect(observed.portablePass).toBe("true");
  expect(observed.boundaryPass).toBe("true");
  expect(observed.calibrationPass).toBe("true");
  expect(observed.derivedTotals).toBe("true");
  expect(observed.summary).toContain(":portable");
  expect(observed.summary).toContain(":capability-specific");
  expect(observed.summary).toContain(":inventory-only");
  expect(observed.interpreted).toBe(observed.probeExpected);
  expect(observed.compiled).toBe(observed.probeExpected);
});

test("browser Wasm keeps the specs-owned MapEntry calibration in evaluator/bytecode parity", async ({ page }) => {
  const corpus = await readFile(fileURLToPath(corpusUrl), "utf8");
  const resources = await readFoundationResources();

  await page.goto("/rust/web/index.html");
  const observed = await page.evaluate(async ({ resources, corpus }) => {
    const { start } = await import(
      "/rust/web/packages/browser/dist/hara-wasm-full/hara.mjs"
    );
    const hara = await start({ resources });
    for (const namespace of Object.keys(resources)) hara.require(namespace);
    hara.eval("(ns user)");
    hara.eval(corpus);
    const probe = JSON.parse(
      String(
        hara.eval(
          "(get (get foundation-calibration-snippets :map-entry-boundary) :source)"
        )
      )
    );
    const expected = String(
      hara.eval(
        "(get (get foundation-calibration-snippets :map-entry-boundary) :expected)"
      )
    );
    const calibration = String(
      hara.eval(
        "(first (filter (fn [case] (= :map-entry-boundary (:id case))) (foundation-calibration-results)))"
      )
    );
    const pass = String(
      hara.eval(
        "(get (first (filter (fn [case] (= :map-entry-boundary (:id case))) (foundation-calibration-results))) :pass)"
      )
    );
    return {
      calibration,
      expected,
      pass,
      interpreted: String(hara.eval(probe)),
      compiled: String(hara.evalBytecode(hara.compileBytecode(probe)))
    };
  }, { resources, corpus });

  expect(observed.pass, observed.calibration).toBe("true");
  expect(observed.interpreted).toBe(observed.expected);
  expect(observed.compiled).toBe(observed.expected);
});
