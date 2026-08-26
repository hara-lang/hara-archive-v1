# Google Drive rich HTA provider

`provider/provider.wasm` is the portable `:require` façade for
`hara/filesystem-google-drive`. The trusted host owns the configured Drive or
shared-drive root ID, OAuth token provider, REST fetch, and cancellation.

Resolution uses stable Drive IDs and exact child names. Duplicate names fail
with `file/ambiguous-path`; shortcuts are reported but never followed. Binary
files use exact media downloads/uploads. Google Workspace documents are
unsupported by default and can only be read through an explicitly configured
source-MIME to export-MIME map.

Folder listing uses Drive page tokens behind opaque provider tokens. Mutations
fence against `headRevisionId`; delete moves the item to Drive trash. Root IDs,
tokens, private permissions, and API URLs stay in the host.

Focused validation:

```text
node --test providers/google-drive/hta/host.test.mjs providers/google-drive/hta/route.test.mjs
```
