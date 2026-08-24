# `std.foundation` variadic and arity inventory

## Purpose

This is the source-backed arity reference for translating Clojure collection
and core call shapes into Hara. It is descriptive, not a second API manifest:
the canonical implementation remains
[`core/lib/src/std/foundation.hal`](../../lib/src/std/foundation.hal), and
runtime-owned forms remain defined by the Rust evaluator and primitive
implementation. `assoc` and `dissoc` are deliberately not runtime-owned:
their public owners are the Foundation Vars below, and those Vars dispatch
through the protocol registry.

Do not infer a Hara call shape from a Clojure Var name. First determine whether
the call resolves to a Foundation Var, a runtime intrinsic, a protocol method,
or a native forwarding shim. “Variadic” below means a callable accepts a
variable number of arguments; “multi-arity” means it declares separate fixed
arities, which is a different contract.

## Associative collection boundary

These operations are intentionally recorded together because their apparent
Clojure shapes are variadic while their protocol methods are fixed-arity.

| Call surface | Owner and declared contract | Translation consequence | Evidence |
| --- | --- | --- | --- |
| `std.foundation/assoc` | HAL Var with `[value key new-value & more]`; each pair forwards to `IAssoc/assoc` | The qualified Foundation Var accepts one or more key/value pairs and rejects an incomplete tail. The protocol method itself remains fixed at three arguments. | [`foundation.hal:730`](../../lib/src/std/foundation.hal#L730), [`environment.rs:435`](../../rust/src/core/environment.rs#L435), [`foundation_test.hal`](../../lib/test/std/foundation_test.hal) |
| `std.foundation/dissoc` | HAL Var with `[value key & more]`; each key forwards to `IDissoc/dissoc` | The qualified Foundation Var accepts one or more keys. `nil` remains a nil-preserving boundary, while the protocol method itself remains fixed at two arguments. | [`foundation.hal:745`](../../lib/src/std/foundation.hal#L745), [`environment.rs:438`](../../rust/src/core/environment.rs#L438), [`foundation_test.hal`](../../lib/test/std/foundation_test.hal) |
| unqualified `assoc` / `dissoc` | Namespace resolution to the corresponding `std.foundation` Var | There is one public callable pathway: ordinary symbol resolution finds the Foundation Var, which then dispatches through `IAssoc/assoc` or `IDissoc/dissoc`. No evaluator structural arm or runtime callable owns either name. | [`evaluator.rs`](../../rust/src/core/evaluator.rs), [`direct_callable_catalog.rs`](../../rust/src/core/direct_callable_catalog.rs), [`foundation.hal`](../../lib/src/std/foundation.hal) |
| `std.foundation/assoc-in` | HAL Var with `[value keys new-value]` | Nested paths are one vector argument plus the replacement; it is not a variadic key-path form. | [`foundation.hal:747`](../../lib/src/std/foundation.hal#L747) |
| `std.foundation/update` | HAL Var with `[value key function & args]` | The update function receives the current value followed by the variadic tail. | [`foundation.hal:761`](../../lib/src/std/foundation.hal#L761) |
| `std.foundation/update-in` | HAL Var with `[value keys function & args]` | The update function receives the current path value followed by the variadic tail. | [`foundation.hal:768`](../../lib/src/std/foundation.hal#L768) |

## Collection constructors and map composition

| Vars | Contract | Boundary and behavior | Evidence |
| --- | --- | --- | --- |
| `list`, `vector`, `tuple`, `hash-map`, `hash-set` | Variadic `& values`/`& entries` | These are Foundation wrappers over `std.native.Base`. `hash-map` consumes alternating key/value entries; the other constructors consume values. | [`foundation.hal:46-72`](../../lib/src/std/foundation.hal#L46-L72), [`foundation_test.hal`](../../lib/test/std/foundation_test.hal) |
| `merge` | Variadic `& sources` | Sources are merged left-to-right; later entries win, and `nil` sources are skipped. Zero sources produce an empty map. | [`foundation.hal:778-787`](../../lib/src/std/foundation.hal#L778-L787), [`foundation_test.hal`](../../lib/test/std/foundation_test.hal) |
| `merge-with` | `[function & maps]` | Maps are merged left-to-right; duplicate values are combined by `function`; `nil` maps are skipped. | [`foundation.hal:1521-1542`](../../lib/src/std/foundation.hal#L1521-L1542), [`foundation_test.hal`](../../lib/test/std/foundation_test.hal) |
| `merge-nested` | Variadic `& maps` | Recursive map values are merged; later non-map values replace earlier values. | [`foundation.hal:1544-1553`](../../lib/src/std/foundation.hal#L1544-L1553), [`foundation_test.hal`](../../lib/test/std/foundation_test.hal) |

## Variadic callable Vars

The following public `std.foundation` Vars have a variadic tail or accept a
fully variadic argument vector. Their source declarations are the authority
for minimum arity and result behavior.

| Group | Vars | Source |
| --- | --- | --- |
| Runtime and application | `stream`, `apply`, `swap!`, `invoke-as` | [`foundation.hal:89-93`](../../lib/src/std/foundation.hal#L89-L93), [`foundation.hal:123-126`](../../lib/src/std/foundation.hal#L123-L126), [`foundation.hal:570-575`](../../lib/src/std/foundation.hal#L570-L575), [`foundation.hal:2424-2428`](../../lib/src/std/foundation.hal#L2424-L2428) |
| Permissive predicates | `T`, `F`, `NIL` | [`foundation.hal:357-370`](../../lib/src/std/foundation.hal#L357-L370) |
| Object-first/application helpers | `apply-with`, `tap` | [`foundation.hal:396-413`](../../lib/src/std/foundation.hal#L396-L413) |
| Composition | `concat`, `comp`, `partial`, `juxt` | [`foundation.hal:432-475`](../../lib/src/std/foundation.hal#L432-L475) |
| Sequence sources | `interleave`, `map`, `zip`, `mapv` | [`foundation.hal:950-970`](../../lib/src/std/foundation.hal#L950-L970), [`foundation.hal:1051-1057`](../../lib/src/std/foundation.hal#L1051-L1057), [`foundation.hal:1089-1097`](../../lib/src/std/foundation.hal#L1089-L1097). `zip` produces pair vectors in the current iterator boundary. |
| Numeric and bitwise operations | `bit-and`, `bit-or`, `bit-xor`, `bit-shift-left`, `bit-shift-right`, `min`, `max` | [`foundation.hal:1130-1164`](../../lib/src/std/foundation.hal#L1130-L1164), [`foundation.hal:1284-1300`](../../lib/src/std/foundation.hal#L1284-L1300) |
| Equality and set algebra | `distinct?`, `union`, `intersection`, `difference` | [`foundation.hal:1448-1458`](../../lib/src/std/foundation.hal#L1448-L1458), [`foundation.hal:1707-1733`](../../lib/src/std/foundation.hal#L1707-L1733) |

`comp`, `apply-with`, and the sequence helpers are also multi-arity APIs:
their fixed forms return a reusable function or transform, while their
variadic forms perform the operation immediately. Preserve that distinction
when translating a call site.

## Arity-sensitive non-variadic neighbors

These common collection/core operations are not part of the variadic HAL Var
list, but their call shapes still need source/runtime evidence:

| Operation | Contract | Evidence |
| --- | --- | --- |
| `get` | Foundation wrapper over `ILookup/lookup` with two- and three-argument forms | [`foundation.hal:45-58`](../../lib/src/std/foundation.hal#L45-L58) |
| `nth` | Foundation wrapper over `INth/nth` with exactly two arguments | [`foundation.hal:60-66`](../../lib/src/std/foundation.hal#L60-L66) |
| `conj` | Foundation wrapper over `IConj/conj` with a collection and one or more values | [`foundation.hal:79-89`](../../lib/src/std/foundation.hal#L79-L89) |
| `cons` | Foundation wrapper over `ICons/cons` with exactly an item and a collection | [`foundation.hal:91-96`](../../lib/src/std/foundation.hal#L91-L96) |
| `filter`, `take`, `drop`, `mapcat`, `keep`, `every?`, `any?` | Foundation multi-arity forms: unary form returns a reusable transform; collection form executes it | [`foundation.hal:958-1087`](../../lib/src/std/foundation.hal#L958-L1087) |

## Macro call shapes

Macros are not function Vars and are therefore excluded from the callable
inventory above. Their variadic forms still matter to source translation:
`->`, `->>`, `case`, `some->`, `some->>`, `doto`, `if-not`, `when`, `if-let`,
`when-let`, `doseq`, `dotimes`, `while`, `cond->`, `cond->>`, `with-ns`,
`with-out-string`, `with-close`, `intern-in`, `intern-all`,
`with-template-meta`, `template-vars`, and `template-entries`. Their source
signatures are in [`foundation.hal:2378-2667`](../../lib/src/std/foundation.hal#L2378-L2667).

## Maintenance rule

When a Foundation or runtime collection operation changes, update this
inventory from the owning source and add or update a behavioral assertion in
the path-matched Foundation or runtime test. In migration code, record the
resolved owner and call form rather than rewriting a call merely to resemble a
familiar Clojure idiom.
