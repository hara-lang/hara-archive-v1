# @hara-lang/hta

Portable HTA0 codecs, manifests, browser host contexts, provider transports,
and the browser-Wasm restricted-sandbox adapter.

```js
import { decodeHta, encodeHta } from "@hara-lang/hta";
import { BrowserWasmSandbox } from "@hara-lang/hta/sandbox";
import { serveNodeProvider } from "@hara-lang/hta/provider/node";
import { createBrowserProvider } from "@hara-lang/hta/provider/browser";
```

The provider helpers accept an async `(operation, arguments, context) => value`
function and implement the same lifecycle for their respective runtime.
`createBrowserProvider` is the provider-side contract; the runtime-owned
`@hara-lang/hta/worker` is the browser worker entry. Node uses process framing
for transport, but both runners expose the observational
`hara.hta.provider.event/0-alpha` lifecycle trace when instrumentation is
requested. The trace covers start, call entry/terminal status, cancellation,
release, and exactly-once shutdown without exposing returned values or opaque
handle identities.

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
package target. The `@hara-lang/hta/shared-worker` export is the shared
transport used by the browser kernel broker. It supports the same generic
provider backend for shared-worker consumers while retaining the raw Wasm path
for the kernel; it is not a package provider target.
