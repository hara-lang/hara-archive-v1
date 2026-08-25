# WebDAV HTA browser provider

This directory is the portable browser route for the `hara/filesystem-webdav`
provider. `extension.edn` declares the HTA ABI, exports, capabilities, provider
artifact, and the only host-call service the provider may use.

The generic HTA worker owns transport, logical-path normalization, capability
checks, paging, revision fencing, mutation semantics, and provider lifecycle.
The provider owns the WebDAV implementation; the browser host owns the WebDAV
root, authentication headers, `fetch`, redirects, response bounds, DAV href
confinement, and request cancellation. Root URLs and credentials are
constructor-only JavaScript authority and are never returned through
descriptors or HTA values.

## Host integration

Create one trusted host adapter and add its `hostCalls` entries to the browser
package loader configuration that activates this route:

```js
import { createWebdavFetchHost } from "@hara-lang/fs-webdav";

const webdavHost = createWebdavFetchHost({
  rootUrl: "https://dav.example.test/root/",
  headers: () => ({ Authorization: `Bearer ${readAccessToken()}` }),
  capabilities: [
    "read", "entries", "write", "mkdir", "delete", "copy", "move",
    "revision-check"
  ]
});

const hostCalls = {
  ...webdavHost.hostCalls
};
```

The adapter defaults to read and entries only. Writable capabilities must be
explicitly selected by the trusted host. Call `webdavHost.closeAll()` when the
owning application or package-loader scope is disposed.

## Unsupported operations

The first route deliberately does not advertise append, atomic move, or
preserved modified times. Attempts fail with `file/unsupported` rather than
silently weakening the requested operation.

## Focused validation

From `core/rust/web`:

```text
npm run test:webdav-provider
npm run test:webdav-route
npm run test:hta-provider-browser
npm run build:webdav-browser
```
