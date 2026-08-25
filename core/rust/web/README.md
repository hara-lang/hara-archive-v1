# rust/web

Browser-side loaders and UIs for the hara wasm runtimes, served as static
assets. The pages deploy copies the runtime-facing pieces under
`site-build/rust/` (see `.github/workflows/pages.yml`).

## Pieces

- `packages/browser/` — the publishable `@hara-lang/browser` SDK. It wraps the
  wasm-bindgen runtime, exposes `Hara.start()` for ESM and CDN script embeds,
  and carries the generated HAL catalog in its release bundle.
- `packages/hta/` — the publishable `@hara-lang/hta` package: HTA0 codecs,
  browser hosts, and reusable Node/browser provider transports. It is the
  only supported HTA browser package entry point.
- `packages/hta/worker.js` — the raw HTA worker: `HtaContext` drives one
  raw wasm instance (`rust/raw`) inside a Web Worker over the `HTA0` binary
  wire format, with handles and the promise-provider contract
  (`specs/01-lang/008-hta/draft/hal-hta-contract.md`).
- `index.html` / `playground.js` — the wasm-bindgen playground page
  (in-browser runtime plus Noir proving).
- `studio/` — the shared studio environment:
  - `broker.js` — kernel broker; one kernel = one Web Worker running one raw
    HTA wasm instance (mirrors the JVM `SessionKernel`).
  - `live-session-controller.js` — the Studio-side request boundary for the
    backend-neutral live-session protocol. It computes source revisions,
    applies generation/revision fences, gates controls by capabilities, and
    validates monotonic replies without exposing evaluator sessions.
  - `host-services.js` — generic host services for kernels (`store/*` over
    IndexedDB, `http/get`).
  - `boot.js` + `hal/` — the bootstrap model: kernels boot from hara
    resources (`store`, `fs`, `space`, `boot`) evaluated inside the kernel
    itself.
  - `ui.js` — `mountStudio`, a framework-free studio UI (file tree, editor,
    REPL, space/kernel switchers); styling in `studio.css`.

  Mounted by the hara-www studio page (`overrides/studio.html`) and
  the greenways-os DevTools panel.

## Test

    npm run test:hta       # HTA loader unit tests
    npm run test:studio    # studio node tests (host services, broker, hal, UI)
    npm run test:browser   # playwright browser smoke

The `studio-hal` and `studio-broker` real-wasm integration tests need the
raw wasm artifact (`bash scripts/runtime/build-hara-wasm-raw` from the repo root)
and self-skip without it.
