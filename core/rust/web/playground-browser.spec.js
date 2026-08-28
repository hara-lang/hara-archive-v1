import { test, expect } from "@playwright/test";

test("wasm playground loads its packaged runtime without failed requests", async ({ page }) => {
  const failures = [];
  page.on("response", (response) => {
    if (response.status() >= 400) {
      failures.push(`${response.status()} ${response.url()}`);
    }
  });

  await page.goto("/rust/web/index.html");
  await expect(page.locator("#result")).toContainText("ready");
  expect(failures).toEqual([]);
});
