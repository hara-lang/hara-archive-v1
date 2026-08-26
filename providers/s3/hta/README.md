# S3 rich HTA provider

`provider/provider.wasm` is the portable `:require` façade for
`hara/filesystem-s3`. The trusted Node host owns the bucket/prefix scope,
credential resolver, optional request signer, HTTP client, and cancellation
controllers.

The route maps objects to files and common-prefix results to virtual
directories. It supports exact-byte reads and writes, delimiter-paged entries,
server-side copy, and guarded copy-then-delete moves. Material directories,
append, atomic move, symlinks, recursive delete, and modified-time preservation
are rejected unless a later host explicitly proves and advertises them.

Construct the host with a fixed `bucket`, optional `prefix`, and trusted
`signRequest` callback. Credentials and endpoint authority never cross the
Wasm boundary.

Focused validation:

```text
node --test providers/s3/hta/host.test.mjs providers/s3/hta/route.test.mjs
```
