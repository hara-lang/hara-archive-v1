import { existsSync } from "node:fs";
import { expect, test } from "@playwright/test";

// Studio smoke: boots the shared UI against the real raw wasm and evals in
// the ROOT kernel. Skipped when the raw wasm artifact has not been built.
const wasmPath = new URL("../raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-vm.wasm", import.meta.url);
test.skip(!existsSync(wasmPath), "raw wasm artifact not built");

test("studio boots live and evals (+ 1 2) in the ROOT kernel", async ({ page }) => {
  await page.goto("/rust/web/studio-browser.html");
  await expect(page.locator('[data-hara-studio="runtime-status"]')).toHaveAttribute("data-state", "live", { timeout: 60000 });
  await expect(page.locator('[data-hara-studio="project-bar"]')).toBeVisible();
  await expect(page.getByText("HARA STUDIO", { exact: true })).toHaveCount(0);
  await expect(page.getByText("ENV/01 · LIVE WASM", { exact: true })).toHaveCount(0);
  await page.locator('[data-hara-studio="runtime-status"]').click();
  await expect(page.locator('[data-hara-studio="runtime"]')).toHaveText("LIVE");
  await expect(page.locator('[data-hara-studio="kernel"]')).toHaveText("ROOT");
  await page.getByRole("button", { name: "Show console" }).click();
  await page.fill('[data-hara-studio="repl-input"]', "(+ 1 2)");
  await page.press('[data-hara-studio="repl-input"]', "Enter");
  await expect(page.locator('[data-hara-studio="repl-log"]')).toContainText("=> 3");
});

test("InstaREPL results open their retained structured trace", async ({ page }) => {
  await page.goto("/rust/web/studio-browser.html");
  await expect(page.locator('[data-hara-studio="runtime-status"]')).toHaveAttribute("data-state", "live", { timeout: 60000 });
  await page.evaluate(async () => {
    const source = "(defn observed [x] (+ x 1))\n(observed 41)";
    await window.studio.writeText("/trace-demo.hal", source);
    await window.studio.refreshFiles();
    await window.studio.openFile("/trace-demo.hal");
  });
  await page.getByRole("button", { name: "Toggle InstaREPL" }).click();
  await expect(page.locator(".hara-studio-insta-result.is-ok")).toHaveCount(2);
  await page.locator(".hara-studio-insta-result.is-ok").last().click();
  await expect(page.locator(".hara-studio-trace-summary")).toContainText("OK");
  await expect(page.locator(".hara-studio-trace-node.is-operation")).toContainText("observed");
  await expect(page.locator(".hara-studio-trace-result")).toHaveText("42");
});

test("compact studio chrome has no horizontal overflow on phone", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/rust/web/studio-browser.html");
  await expect(page.locator('[data-hara-studio="runtime-status"]')).toHaveAttribute("data-state", "live", { timeout: 60000 });
  await expect(page.locator(".hara-studio-mobile-tabs")).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
