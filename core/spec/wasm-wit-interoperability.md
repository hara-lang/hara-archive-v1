# WIT interoperability

The canonical Hara interface remains the `.hal` file. WIT is a bounded
interoperability input and output format; it does not replace Hara schemas,
ownership, capabilities, error conventions, or the HTA lifecycle.

## Import

```text
hara extension wit-import api.wit --out api.hal \
  --module api.wasm --namespace demo.api --world api-world --strict
```

Import emits a deterministic `hara.wasm-interface/0-alpha` skeleton. Use
`--ir path.edn` to also write the normalized `hara.wasm-wit/0-alpha` IR.
Without `--strict`, the command still emits the skeleton but prints every
lossy or unsupported mapping. Strict mode fails before writing output.

Scalar WIT functions retain the native/direct `:import` route. Memory-backed
strings and byte lists remain direct `memory.v1` candidates. Resources,
options, results, non-byte lists, asynchronous declarations, and host imports
are marked as requiring the HTA `:require` route; they never fall back to a
native import or to an implicit JVM/.NET artifact.

## Projection

```text
hara extension wit-project api.hal --out api.wit --strict
```

Projection emits only the subset represented exactly by the Hara interface.
Named records, variants, handles, callbacks, error mappings, capabilities, and
asynchronous exports require their canonical Hara definitions and are reported
as unsupported rather than guessed.

Generated JavaScript, Rust, Java, and C wrappers are optional export backends;
they are never required by Hara's runtime path.

## Reproducibility and versioning

Import output is content-addressed by the WIT source and generated interface
bytes. The manifest schema is `hara.wasm-wit-manifest/0-alpha` and records the
source/interface digests, origin, route, and diagnostics. Re-running a
conversion with the same source and options produces byte-identical interface
and IR output.

Schema revisions are additive while the `0-alpha` contract is in development.
An incompatible change must introduce a new schema revision and retain the
older reader for its supported lifetime. Unsupported revisions fail closed;
there is no implicit downgrade or cross-route fallback.
