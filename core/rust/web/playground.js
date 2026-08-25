import init, { Runtime, version, target_profile } from "./pkg/hara_wasm.js";

const source = document.querySelector("#source");
const result = document.querySelector("#result");
const button = document.querySelector("#run");

await init();
const runtime = new Runtime();
runtime.install_memory_file_provider("/browser");
runtime.install_loopback_socket_provider();
result.textContent = `${version()} / ${target_profile()}\nfile=${runtime.file_supported()} socket=${runtime.socket_supported()}\nready`;
button.addEventListener("click", () => {
  try { result.textContent = runtime.eval(source.value); }
  catch (error) { result.textContent = `error: ${error}`; }
});
