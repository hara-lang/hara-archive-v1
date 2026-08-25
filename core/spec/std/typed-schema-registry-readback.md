# `std.typed` registry catalogue read-back

Status: draft implementation contract
Owner issue: `hara-lang/hara#903`
Registry fixture:
`hara-lang/hara-specs-registry/01-lang/011-typed-catalog/draft/conformance/catalog-v2.json`

## Purpose

A published `std.typed` catalogue is not executable merely because the registry
accepted its JSON envelope. Hara must read the checked-in document, reconstruct
the identified schemas through the canonical `std.typed.catalog` implementation,
and compare every semantic identity and dependency claim before the catalogue
can be supplied to HBC, HBX, package, or runtime admission.

This boundary is implemented by `std.typed.catalog.document`.

## Public operations

```clojure
(std.typed/catalog-document-verify document)
(std.typed/catalog-document-verify-json json-source)
```

A successful call returns:

```clojure
{:catalog <verified SchemaCatalog>
 :verification
 {:status :verified
  :catalog/format "std.typed.catalog/2"
  :catalog/hash-epoch "std.typed.schema/catalog-v2"
  :catalog/component-epoch "std.typed.catalog/component-v2"
  :catalog/provenance ...
  :catalog/document-digest "sha256:..."
  :catalog/entry-count ...
  :catalog/component-count ...
  :catalog/coordinates [...]
  :catalog/component-order [...]}}
```

No catalogue value is returned when any check fails.

## Verification sequence

The document reader performs the following work in order:

1. Require the document, hash, and component epochs.
2. Validate immutable source provenance and canonical source paths.
3. Parse each `schema/form` and `schema/normal` as one HAL/EDN data value.
4. Convert all coordinates to exact `[:schema id hash]` values.
5. Construct one `std.typed.catalog/SchemaCatalog` using the supplied hashes as
   assertions.
6. Recompute and compare every normalized schema and direct exact dependency.
7. Recompute strongly connected components, component identities, recursion
   evidence, component dependencies, and dependency-first component order.
8. Return the catalogue and deterministic verification report atomically.

The document layer does not contain another schema grammar, semantic hash
algorithm, dependency traversal, or strongly connected component
implementation. Those remain owned by `std.typed.catalog`.

## Exact links

Execution links remain exact coordinates:

```clojure
[:schema :catalog.fixture/profile "sha256:..."]
```

There is no latest-version projection in the admitted catalog. Execution uses
the exact coordinate directly.

## Digest ownership

`catalog/document-digest` authenticates the registry document's canonical JSON
projection. Its stable-JSON algorithm remains owned and checked by
`hara-specs-registry`.

Hara requires a canonical SHA-256 digest field and carries it into the
verification report, but deliberately does not duplicate the registry's JSON
canonicalisation algorithm. Hara independently recomputes the semantic schema
hashes, dependencies, and components that it owns.

## Failure contract

Failures use:

```clojure
{:type :std.typed.catalog/invalid-document
 :finding/type :std.typed.catalog.document/...}
```

Important findings include:

- `format-unsupported`
- `hash-epoch-unsupported`
- `component-epoch-unsupported`
- `catalog-rejected`
- `schema-normal-mismatch`
- `schema-dependencies-mismatch`
- `component-evidence-mismatch`
- `component-order-mismatch`

When canonical catalogue construction rejects an entry, the wrapper retains the
underlying `:cause/type`, including
`:std.typed.catalog/invalid-catalog`.

## Atomicity

Verification is pure. It builds a local immutable catalogue and returns it only
after all document, identity, dependency, component, provenance, and tooling
checks pass. It does not mutate a runtime registry, install a package, release
an HBC program, perform network I/O, or retain partial catalogue state.

Rust package manifest and archive admission now consume this verified result
before package build, read, install, or activation. The package boundary keeps
the exact coordinates from the verified catalogue. HBC1 remains an exact-link
boundary: linked programs carry exact schema coordinates and do not embed the
registry JSON document.

## Conformance

The permanent `std.typed schema parity` workflow reads the exact checked-in
fixture bytes through three supported paths:

- the native Hara CLI;
- the Rust `Runtime` embedding;
- the Java/Truffle `Context` embedding.

The corpus covers successful exact read-back plus rejection of:

- a stale supplied semantic hash;
- a stale exact dependency;
- forged recursive-component evidence;
- an unsupported semantic hash epoch.

The package conformance contract is published in
`hara-lang/hara-specs-registry/02-platform/000006-package/draft/conformance/catalog-admission.edn`.
It requires package admission to read the same checked-in registry fixture,
verify its provenance and exact coordinates, and reject stale or unresolved
catalogue evidence before producing an installable artifact.

The existing HBC1 native admission tests remain responsible for rejecting
missing exact links before linked-program release.
