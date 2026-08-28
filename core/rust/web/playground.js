import { start } from "./packages/browser/dist/hara-wasm-full/hara.mjs";

const source = document.querySelector("#source");
const result = document.querySelector("#result");
const button = document.querySelector("#run");

const hara = await start();
result.textContent = "Hara browser / full runtime\nready";
button.addEventListener("click", () => {
  try { result.textContent = hara.eval(source.value); }
  catch (error) { result.textContent = `error: ${error}`; }
});
