import { test, expect } from "@playwright/test";

test("Chromium/Wasm matches the native Rust and Java instrumentation reports", async ({ page }) => {
  await page.goto("/rust/web/index.html");
  const reports = await page.evaluate(async () => {
    const [corpus, rust, java] = await Promise.all([
      fetch("/spec/instrumentation-conformance.json").then((response) => response.json()),
      fetch("/target/instrumentation-rust-a.json").then((response) => response.json()),
      fetch("/target/instrumentation-java-a.json").then((response) => response.json())
    ]);
    const { start } = await import("/rust/web/packages/browser/dist/hara-wasm-full/hara.mjs");
    const hara = await start();
    const first = hara.instrumentationConformance(corpus);
    const second = hara.instrumentationConformance(corpus);
    return { first, second, rust, java };
  });

  expect(reports.first).toEqual(reports.second);
  expect(reports.first.runtime).toBe("wasm");
  expect(reports.first.cases).toEqual(reports.rust.cases);
  expect(reports.first.cases).toEqual(reports.java.cases);
});
