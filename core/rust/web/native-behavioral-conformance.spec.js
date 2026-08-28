import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { test, expect } from "@playwright/test";
import { readFoundationResources } from "./conformance-bootstrap.js";

const corpusUrl = new URL(
  "../../../../hara-specs-registry/01-lang/001-language/draft/conformance/fixtures/native_behavioral.hal",
  import.meta.url
);

test("browser Wasm consumes the specs-owned native behavioral corpus", async ({ page }) => {
  const corpus = await readFile(fileURLToPath(corpusUrl), "utf8");
  const resources = await readFoundationResources();

  await page.goto("/rust/web/index.html");
  const observed = await page.evaluate(async ({ corpus, resources }) => {
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

    const valid = String(hara.eval(`${corpus}\n(native-corpus-valid?)`));
    const keys = String(hara.eval(`${corpus}\n(native-method-keys)`));
    const methods =
      keys.match(/[A-Z][A-Za-z0-9]*\/[A-Za-z0-9?!+*._-]+/g) ?? [];
    const allResults = String(hara.eval(`${corpus}\n(native-method-results)`));
    const boundaryPass = String(
      hara.eval(
        `${corpus}\n(every? (fn [case] (= true (get case :pass))) (native-boundary-results))`
      )
    );
    const profilePass = String(
      hara.eval(
        `${corpus}\n(let [report (native-profile-report)] (and (= 0 (get report :failed)) (= (+ (get report :passed) (get report :failed) (get report :skipped)) (+ (get report :portable) (get report :capability-specific) (get report :inventory-only)))))`
      )
    );
    const summary = String(
      hara.eval(`${corpus}\n(native-classification-summary)`)
    );
    const probe = JSON.parse(
      String(
        hara.eval(
          `${corpus}\n(get (get native-calibration-snippets :evaluator-compiler) :source)`
        )
      )
    );
    const probeExpected = String(
      hara.eval(
        `${corpus}\n(get (get native-calibration-snippets :evaluator-compiler) :expected)`
      )
    );
    const interpreted = String(hara.eval(probe));
    const artifact = hara.compileBytecode(probe);
    const compiled = String(hara.evalBytecode(artifact));
    return {
      valid,
      methods,
      allResults,
      boundaryPass,
      profilePass,
      summary,
      probeExpected,
      interpreted,
      compiled
    };
  }, { corpus, resources });

  expect(observed.valid).toBe("true");
  expect(observed.methods.length).toBeGreaterThan(0);
  expect(new Set(observed.methods).size).toBe(observed.methods.length);
  expect(observed.allResults).not.toContain(":pass false");
  expect(observed.allResults.match(/:pass true/g)?.length ?? 0).toBe(
    observed.methods.length
  );
  expect(observed.boundaryPass).toBe("true");
  expect(observed.profilePass).toBe("true");
  expect(observed.summary).toContain(":portable");
  expect(observed.summary).toContain(":capability-specific");
  expect(observed.summary).toContain(":inventory-only");
  expect(observed.interpreted).toBe(observed.probeExpected);
  expect(observed.compiled).toBe(observed.probeExpected);
});
