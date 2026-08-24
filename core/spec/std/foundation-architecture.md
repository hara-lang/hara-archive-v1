# Current `std.foundation` architecture

This document describes the implementation boundary represented by Hara's registered standard-library inventory. It is intentionally narrower than historical Foundation plans and annex drafts.

## Loadable Foundation namespaces

The current public Foundation family is:

```text
std.foundation
std.foundation.bytes
std.foundation.coroutine
std.foundation.pretty
std.foundation.promise
std.foundation.string
```

`core/rust/bootstrap.namespaces` selects the Foundation source namespaces embedded in production Java and Rust runtimes. Within each selected namespace, the canonical `.hal` source owns its public Vars: evaluating a public definition interns it in that namespace, and bytecode generation discovers the same definitions from source. Inventories and generated manifests describe that surface; they do not gate individual symbols. The broader `standard-library.namespaces` catalog remains development/package input, so membership there alone does not make a source part of the bootstrap.

The root `std.foundation` namespace owns the portable value layer: composition, collections, sequence operations, set algebra, metadata, references, macros, structural traversal, regular-expression helpers, and the small language-level helpers automatically referred into ordinary namespaces. Regular-expression ownership remains root-level: `regexp`, `re-pattern`, `re-find`, `re-matches`, `re-replace`, and `re-split` are portable HAL functions, while `regexp?` remains a kernel-installed root predicate.

The five child namespaces provide separately aliased portable/native-backed library surfaces:

| Namespace | Default alias | Role |
| --- | --- | --- |
| `std.foundation.string` | `str` | portable string facade |
| `std.foundation.bytes` | `bytes` | byte values and operations |
| `std.foundation.promise` | `promise` | promises and protocol facade |
| `std.foundation.coroutine` | `co` | coroutine facade |
| `std.foundation.pretty` | `pretty` | document and pretty rendering |

These are source-owned global aliases. Each Foundation namespace declares its
alias with `(:config {:set-global-alias alias})`; the Rust runtime records and
applies those declarations but does not seed `str`, `bytes`, `promise`, `co`,
or `pretty` as native defaults. Explicit `:require` aliases remain available
for local naming. Use `:set-global` for qualified Vars imported by terminal
name. Foundation child library aliases and exclusions belong under `:rename`;
there is no separate `:load-aliases` or `:intrinsics` namespace pathway.

## Native static objects

The schema-v2 API manifest records native static objects separately from loadable namespaces. The current cross-profile runtime configuration includes objects such as `Edn`, `Json`, `RegExp`, `Crypto`, `File`, `Socket`, `Host`, and `Kernel`, backed by `std.native.*` runtime identities. They are available without requiring file-backed `std.native.*` namespaces.

For example:

```hara
(Edn/read "{:a 1}")
(Json/write {"a" 1})
(Crypto/sha256 bytes)
```

The presence of aliases such as `Edn` or identities such as `std.native.Edn` does **not** imply that `std.foundation.edn` or another retired Foundation child is loadable.

`Base` owns representation-level constructor identities that otherwise have no
natural receiver: `list`, `vector`, `vec`, `set`, `tuple`, `hash-map`,
`hash-set`, `atom`, `pointer`, `symbol`, and `keyword`. `vec` and `set` retain
native bulk and identity fast paths. Base also owns `reduced`, `reduced?`,
`apply`, truth coercion, comparison, primitive predicates, and the minimal
reflection operations. Derived helpers such as `pair`, `unreduced`,
`reduce-kv`, `merge`, and `select-keys` are canonical HAL. Foundation implements
`reduce-in` portably through `IReduce`, `IToMutable`, and `IToPersistent`;
`reduce-kv` and `select-keys` use it to retain mutable construction speed.
`reduce-in` is not a Base method because its source is reduced through the
existing source-dispatched protocol.

## Callable ownership

A function symbol always has a defining symbol in addition to its Var
provenance. Rust source functions derive that symbol from their defining
namespace and name; native Rust callables retain the qualified name supplied
by their registry entry, or combine an unqualified registry name with the
active namespace. Java builtins receive the owning `namespace/name`
when they are inserted into a namespace, so aliases and generated exports do
not erase ownership. Anonymous closures are values rather than function
symbols and therefore intentionally have no defining symbol.

`OS` remains the migration direction for the former `std.foundation.os` API, but availability and export shape are runtime-profile concerns to be proven by cross-runtime conformance. A process handle is a runtime value, not an automatic `Process` static-object alias. Neither should be presented as part of the common native-object inventory without profile evidence.

## Higher-level ownership

Functionality above the native substrate belongs to focused portable libraries:

- mounted filesystem paths under `std.fs.path`;
- synchronous direct-style traversal and file operations under `std.fs.walk` and `std.fs`, implemented by internally dereferencing the promise-based `File/*` provider boundary; see [Mounted filesystem APIs](filesystem.md);
- formatting, tables, reports, and terminal presentation under `std.format.*`;
- component lifecycle under `std.lib.component`;
- cryptographic algorithms above native primitives under `std.crypto.*`;
- collection helpers not retained in the root value layer under `std.lib.collection`.

The filesystem split is intentional: `File/*` remains the asynchronous capability surface for explicit promise composition, while `std.fs` is the familiar synchronous facade. No parallel asynchronous `std.fs` compatibility namespace is maintained.

Planned replacements must be recorded as planned rather than presented as implemented.

## Migration and generated API data

`core/spec/std/foundation-migrations.json` records former names, their status, replacement or disposition, rewrite guidance, and evidence.

`scripts/generate_foundation_api_manifest.py` combines:

1. the registered namespace inventory;
2. source-derived public binding data;
3. runtime alias/native-object configuration; and
4. the migration ledger.

The resulting schema-v2 manifest records repository-relative provenance, a pinned source commit, and deterministic surface and migration digests. It is the source consumed by the specification registry and generated documentation. Downstream repositories must not maintain independent handwritten Foundation inventories.

## Test placement

Ordinary tests under `core/lib/test/std/foundation/` correspond only to current Foundation child namespaces. Root Foundation behavior lives in `core/lib/test/std/foundation*_test.hal`. Common native static-object behavior is tested without requiring retired `std.foundation.*` children. Profile-specific OS/process behavior, capability-provider behavior, and higher-level portable-library behavior belong under the runtime or library that owns them.
