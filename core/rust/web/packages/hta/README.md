# @hara-lang/hta

Portable HTA0 codecs, manifests, browser host contexts, provider transports,
and the browser-Wasm restricted-sandbox adapter.

```js
import { decodeHta, encodeHta } from "@hara-lang/hta";
import { BrowserWasmSandbox } from "@hara-lang/hta/sandbox";
import { serveNodeProvider } from "@hara-lang/hta/provider/node";
import { createBrowserProvider } from "@hara-lang/hta/provider/browser";
```

The provider helpers accept an async `(operation, arguments) => value`
function and implement the provider lifecycle for their respective runtime.
`createBrowserProvider` is the provider-side contract; the runtime-owned
`@hara-lang/hta/worker` is the only browser worker entry.

`BrowserWasmSandbox` is a one-shot adapter. It creates one Worker and one Wasm
instance, sends only the closed `sandbox/eval` HTA target, supplies no host-call
or filesystem authority, applies source/output/deadline bounds, rejects live
runtime values, and closes the context and worker after every terminal result.
It deliberately does not fall back to `eval`, `session/eval`, or `ROOT`.

The adapter becomes semantic execution evidence only when paired with a raw
runtime that implements `sandbox/eval` as a transient restricted session. An
ordinary HTA root session is not `hara.mcp-pure/0-alpha`.

The `@hara-lang/hta/worker` export is the runtime-owned generic worker entry
point. It loads either a Wasm HTA adapter or the provider module named by a
package target. The `@hara-lang/hta/shared-worker` export remains the shared
raw-runtime transport used by the browser kernel broker; it is not a package
provider target.
