# IndexedDB rich HTA provider

`provider/provider.wasm` is the portable `:require` façade for
`hara/filesystem-indexeddb`. The trusted host owns IndexedDB database handles,
transactions, quota, schema initialization, and cancellation. Database names
and namespace selection are configuration inputs and are never exposed as
transport authority through the provider descriptor.

The route reuses the transactional IndexedDB filesystem implementation under
`core/rust/web/host` and exposes the same filesystem operation surface as the
other rich HTA providers. Its `entries-page` operation stays paged and its
revision values fence concurrent mutations.

Focused validation:

```text
node --test providers/indexeddb/hta/host.test.mjs providers/indexeddb/hta/route.test.mjs
```
