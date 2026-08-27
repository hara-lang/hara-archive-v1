import { HtaContext } from "./packages/hta/index.js";
const bytes=new Uint8Array(await (await fetch("/rust/raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-vm.wasm")).arrayBuffer());
const worker=new Worker("./packages/hta/worker.mjs",{type:"module"});
const context=new HtaContext({worker,moduleBytes:bytes,hostCalls:{"crypto.hash.sha256/digest":async input=>new Uint8Array(await crypto.subtle.digest("SHA-256",input))}});
window.htaContext=context;
window.htaSmoke=context.call("eval",['(+ 10 (count (deref (std.native.Host/call "crypto.hash.sha256" "digest" [(Bytes/new 97 98 99)]))))']);
