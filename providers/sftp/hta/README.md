# SFTP rich HTA provider

`provider/provider.wasm` is the portable `:require` façade for
`hara/filesystem-sftp`. The trusted Node host receives a credential reference,
an explicit pinned or trusted-known-hosts policy, and an injected connection
factory. It requires the resulting client to report both authenticated and
host-key-verified before the mount is created.

The injected client surface is deliberately SFTP-only: `lstat`, `readFile`,
`writeFile`, `readdir`, `mkdir`, `unlink`, `rmdir`, `copyFile`, `rename`, and
`close`. No shell, `scp`, process execution, ambient SSH identity, or
trust-on-first-use path is used. Browser activation has no direct route and
fails closed unless an application supplies an external trusted host bridge.

Root confinement checks every ancestor and never follows symbolic links.
Append, atomic move, preserved modified times, and revision checks are
advertised only when the negotiated client capability vector includes the
corresponding proof.

Focused validation:

```text
node --test providers/sftp/hta/host.test.mjs providers/sftp/hta/route.test.mjs
```
