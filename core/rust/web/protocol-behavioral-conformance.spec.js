import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { test, expect } from "@playwright/test";
import { readFoundationResources } from "./conformance-bootstrap.js";

const specsRoot = new URL(
  "../../../../hara-specs-registry/01-lang/001-language/draft/conformance/fixtures/",
  import.meta.url
);

function passingResultCount(value) {
  return value.match(/:pass true/g)?.length ?? 0;
}

test("browser Wasm consumes the specs-owned protocol corpora", async ({ page }) => {
  const [behavioral, surface] = await Promise.all([
    readFile(fileURLToPath(new URL("protocol_behavioral.hal", specsRoot)), "utf8"),
    readFile(fileURLToPath(new URL("protocol_surface.hal", specsRoot)), "utf8")
  ]);

  expect(behavioral).not.toMatch(/std\.protocol\.[^\s/]+\/I[A-Z]/);
  expect(surface).not.toMatch(/std\.protocol\.[^\s/]+\/I[A-Z]/);
  const resources = await readFoundationResources();

  await page.goto("/rust/web/index.html");
  const observed = await page.evaluate(async ({ behavioral, surface, resources }) => {
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

    const portable = String(hara.eval(behavioral));
    const capability = String(hara.eval("(capability-protocol-results)"));
    const receivers = String(hara.eval("(protocol-receiver-matrix-results)"));
    const crossCutting = String(hara.eval("(protocol-cross-cutting-results)"));
    const capabilityReceivers = String(
      hara.eval("(protocol-capability-receiver-results)")
    );
    const nativeValues = String(hara.eval("(protocol-native-value-results)"));
    const predicates = String(hara.eval("(protocol-predicate-results)"));
    const compilerProbe =
      "(every? :pass (protocol-receiver-matrix-results))";
    const interpreted = String(hara.eval(compilerProbe));
    const compiled = String(
      hara.evalBytecode(hara.compileBytecode(compilerProbe))
    );
    const protocolSurface = String(hara.eval(surface));
    return {
      portable,
      capability,
      receivers,
      crossCutting,
      capabilityReceivers,
      nativeValues,
      predicates,
      interpreted,
      compiled,
      protocolSurface
    };
  }, { behavioral, surface, resources });

  for (const result of [
    observed.portable,
    observed.capability,
    observed.receivers,
    observed.crossCutting,
    observed.capabilityReceivers,
    observed.nativeValues,
    observed.predicates,
    observed.protocolSurface
  ]) {
    expect(result).not.toContain(":pass false");
    expect(passingResultCount(result)).toBeGreaterThan(0);
  }
  expect(observed.interpreted).toBe("true");
  expect(observed.compiled).toBe("true");
});
