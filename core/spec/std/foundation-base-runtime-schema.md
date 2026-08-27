# Foundation Base Shims, Runtime Types, and Typed Schemas

## Status

This document defines the portable Foundation contract. Implementations may use
different host representations, but Java, Rust, interpreted source, and bytecode
must expose the same observable values and errors.

## Foundation and native ownership

`std.foundation` owns the portable root API. Operations implemented by the
runtime live on `std.native.Base`; Foundation exposes explicit forwarding
functions marked `^{:inline true}`. The metadata is a request, not a handwritten
rewrite rule: the compiler validates that the function body is a transparent
forwarder and derives its target from that body. Invalid inline declarations are
compile errors. Direct calls may lower like macros, while the Var remains an
ordinary first-class function for indirect calls.

Adding a portable operation to `Base` requires a corresponding Foundation shim.
`Arr` and `Obj` retain their mutable host-oriented APIs. Specialized persistent
collection constructors and predicates belong to `std.native.Algo`; public
`std.lib.collection` functions remain explicit wrappers such as
`(defn deque? [x] (Algo/deque? x))`.

The baseline bootstrap consists only of the Foundation namespaces required to
load and compile portable source. `std.lib.resp` and other library packages are
package-tier resources. `hara.compiler`, `hara.verify`, and
`hara.transpile.base.*` are not bootstrap dependencies.

Namespace inspection and dynamic evaluation live on `std.native.Runtime`; the
non-loading symbol lookup primitive lives on `std.native.Base`. Foundation
exposes the same transparent wrappers for `ns-current`, `ns-list`,
`ns-info`, `ns-vars`, `env-snapshot`, `env-module`, `resolve`,
`ns-find`, `ns-create`, `ns-name`, `ns-publics`, `ns-aliases`,
`ns-alias-state`, `intern-var`, `eval-in-ns`, and `eval`. The Foundation
`resolve` shim forwards to `Base/resolve`; it observes only Vars already
materialized in the registry and never loads a namespace or package.
Dynamic form evaluation remains Runtime-owned: `Runtime/eval` evaluates one
form value in the current namespace, while `Runtime/eval-in` evaluates a
collection of form values in an existing namespace. Java and Rust must expose
identical methods and evaluation behavior.

The same inline-forwarding rule applies throughout the embedded Foundation
family. Transparent shims over `Maths`, `Num`, `Bits`, `String`, `Bytes`,
`Promise`, `Coroutine`, and protocol methods carry `:inline true`, a public
docstring, and schema metadata. A wrapper that reorders arguments, supplies a
default, normalizes a result, performs capability policy, or composes more than
one call is not a transparent shim and must remain ordinary HAL.

`std.foundation/time-ms` transparently forwards to `OS/time-ms` and returns
Unix wall-clock milliseconds. It is suitable for timestamps, but may jump when
the host clock is corrected. `std.foundation/time-ns` transparently forwards to
`OS/time-ns` and returns a runtime-local monotonic counter expressed in
nanoseconds. Only differences between values from the same process or browser
worker are meaningful; the unit does not imply nanosecond precision. These two
integer primitives are the complete native time surface. Calendar, duration,
formatting, and parsing behavior belongs in portable HAL libraries.

## Base surface

Base includes representation-level constructors, reduction boxing/unboxing,
numeric predicates, `apply`, `satisfies?`, `special-symbol?`, `type`, and
`instance?`. `tuple` accepts zero through eight values. `vec` and `set` use
bulk native construction and return an existing vector or persistent set
unchanged. Truth coercion, comparison, and all derived predicates are
canonical HAL source. The transparent `unreduced` Foundation Var may still
inline to `Base/unreduced`, but it remains a Foundation-owned public name.
`pair`, `pair?`, `not-nil?`, `false?`, `true?`, `fn?`, `reduce`, `reduce-kv`,
`merge`, and `select-keys` are also source definitions. `reduce-in` remains a
portable Foundation algorithm because its protocol composition is not a
primitive runtime operation; `reduce-kv` and `select-keys` use it so
mutable-capable destinations retain the fast construction path.

## Runtime type values

`type` returns flat keywords:

- Native values use `:std.native.<Name>`, including `RegExp`, `Tuple`,
  `Promise`, `Coroutine`, `Namespace`, `NativeType`, `StructType`,
  `MutableType`, and `SchemaType`.
- A named struct or mutable instance uses `:<declaring-ns>.<Type>`, for example
  `:geometry.Point`.
- A native descriptor such as `Base` has type `:std.native.NativeType`.

`instance?` accepts a generated struct/mutable descriptor or a concrete native
descriptor that declares an `instance?` method. It rejects operational native
descriptors. Defining a struct or mutable does not generate
`<Type>/instance?`; generic `instance?` is the sole named-type predicate.

A loaded namespace and an installed alias may be referenced as values without
causing an implicit load. Resolution precedence is lexical binding, Var, loaded
namespace or alias, then unbound-symbol error.

## Pull streams

`Stream` is a native, asynchronous, unidirectional pull source. It implements
`IStream/next` and `IClose/close`; `(type stream)` is `:std.native.Stream`.
`Stream/next` returns a Promise which fulfills with one structured Hara value,
or `nil` at end-of-stream. Only one pull may be pending. Closing is idempotent
and a closed stream produces `nil`.

`std.foundation/stream` is the ordinary language constructor over
`Stream/generate`, and `std.foundation/stream?` recognises the native stream
contract. The constructor owns a private
coroutine, supplies constructor arguments only on its first resume, exposes
yielded values one at a time, and discards the coroutine's final return value.
Because `nil` denotes EOF, yielding `nil` rejects the pull with
`stream/nil-item` and closes the stream. Generator errors reject the active
pull and close the stream. The namespace is deliberately absent from the
Foundation bootstrap bundle.

Foundation iterators are synchronous: `iter-next` either returns immediately
or the iterator is exhausted. `std.stream.async/from-iterator` is the explicit
one-way bridge into Promise-based pulling. `unfold` accepts a direct or
promised step result of `[item next-state]`, with `nil` ending the stream.
`map`, `filter`, and `take` are lazy streams; `reduce` and `collect`
are Promise-returning terminals. Composed streams own and always close their
upstream source on EOF, error, early termination, or explicit close.

A stream is not duplex. Duplex transports compose a readable `IStream` with a
separate write operation; for example, a WebSocket exposes inbound messages as
a stream and outbound messages through `WebSocket/send`. Stream, coroutine,
and transport handles are worker-local and cannot cross session, HTA, snapshot,
or worker serialization boundaries.

The `std.stream.duplex` Hara namespace composes these protocols as a regular
Hara value. There is no boxed native Duplex type. Rust and Java implement the
individual stream, write, close, abort, and lifecycle protocol boundaries;
the portable layer owns their composition.

`std.stream.duplex/from-process` composes `Process/stdout-stream`,
`Process/write`, `Process/close-input`, and `Process/kill`; stderr remains
independently observable through `Process/stderr-stream`.
`std.stream.duplex/from-socket` composes `Socket/receive-stream`, `Socket/send`,
and `Socket/close`; a listening socket is not a Duplex. Sends return Promises,
receive sides preserve the one-pending-pull Stream rule, and explicit close is
idempotent.

Duplex replaces transport-specific input/output plumbing, but not Relay.
Relay remains the portable layer for codecs and framing, serialized or
correlated exchanges, timeouts, pending-request dispatch, and unsolicited
events over a Duplex.

## Schema values and Var contracts

`schema` is a transparent Foundation wrapper over `Schema/compile`, which
compiles schema data into an immutable `SchemaType`. It accepts:

- raw shorthand data such as `[:map [:name :str]]` or `[:int]`;
- canonical normalized data such as `{:kind :map :fields [...]}`;
- retained longhand input such as `{:kind :map :children [...]}`, which is
  immediately converted to the canonical normalized form;
- an existing `SchemaType` (idempotently);
- a Var whose contained value is raw schema data or a `SchemaType`.

`(schema #'description)`, `(schema description)`, and `(schema [:int])` are
structurally equal when `description` contains `[:int]`. Only the Var form has
that Var as its origin; origin is excluded from equality and hashing.
`(schema #'customer-name)` and `(schema customer-name)` are errors when the
value is not schema data. In particular, `schema` never reads a Var's `:schema`
metadata.

`schema-of` transparently wraps `Schema/of`, the contract lookup operation. It accepts only a Var reference:
`(schema-of #'customer-name)` returns the compiled contract snapshot or `nil`.
Passing the function value is an error. Contracts belong to Vars and functions
do not inherit them.

Metadata may point at a schema-data Var:

```hara
(def description [:int])
(defn ^{:schema #'description} customer-name [customer] (:name customer))
```

The compiler resolves and snapshots this contract when the definition is
compiled or reloaded. Later mutation of `description` does not silently change
the already compiled contract.

`Schema/kind`, `Schema/form`, `Schema/ast`, and `Schema/origin` inspect schema
values. `(schema? (schema value))` is the portable schema identity check.
Printing is round-trippable as
`(schema <canonical-short-form>)`. `Schema/ast` returns the portable
normalized map rather than a host compiler-node shape. For every valid
surface schema, portable normalization, native AST inspection, and
re-normalization are structurally equal, and `(schema (Schema/ast value))`
reconstructs a `SchemaType` with the same canonical AST. `Schema/form` and
`Schema/origin` continue to preserve the inspected value's source form and
origin.

`SchemaType` implements `IDeref`. Dereferencing returns the normalized vector
shorthand, independent of the input spelling; for example, both `(schema :int)`
and `(schema [:int])` dereference to `[:int]`, while nested schemas dereference
recursively to forms such as `[:map [:name [:str]]]`.

`Schema/origin` returns provenance, not another schema. Consequently
`(Schema/origin (schema #'customer-name))` is valid as an origin query even
though its result is not a `SchemaType`.

## Conformance requirements

Java and Rust must share tests for tuple arities, all flat type keywords,
native and named `instance?`, namespace values and precedence, inline shim
validation/lowering, schema short/long normalization, Var-origin equality,
schema errors, contract snapshotting, and interpreted/bytecode parity.

## Portable schema registries and named references

`std.typed.registry` owns immutable registry data. A canonical registry has the
portable shape:

```hara
{:registry/type :std.typed.registry/registry
 :registry/namespace demo
 :registry/aliases {model app.model}
 :registry/refers {Id app.model/Id}
 :registry/entries {demo/Node [:map ...]}
 :registry/parents [...]}
```

Registry construction qualifies local entry names. Lookup checks local entries
first, then ordered parents from first to last. Aliases and refers are explicit
data; registry lookup and schema resolution do not evaluate Vars or project
source.

The portable schema layer provides:

```hara
(schema/normalize-with surface registry)
(schema/reference-names surface registry)
(schema/resolve-reference surface registry)
(schema/resolve-recursive surface registry)
(schema/unresolved-references surface registry)
(schema/validate surface value registry)
(schema/valid? surface value registry)
```

Inside a registry context, a bare symbol and `(var Name)` both normalize to a
`:reference`. Unqualified names use the registry namespace, aliases rewrite a
qualified prefix, and refers map an unqualified name directly to a qualified
name.

Recursive resolution expands reachable definitions but leaves a reference at
a recursive edge. Runtime validation follows references lazily. Its cycle key
is `[qualified-reference value-path]`, so structural recursion that consumes a
map or collection is valid, while an alias-only cycle at one value path reports
`:std.typed.schema/cyclic-reference`. Missing definitions report
`:std.typed.schema/unresolved-reference`. Both are ordinary deterministic
findings, not runtime exceptions.

## Canonical portable explanations

`std.typed.explain` converts schema findings into strict portable Failure data.
Every Failure contains all of these fields:

```hara
{:failure/code keyword
 :failure/path vector
 :failure/in vector
 :failure/actual any
 :failure/expected any
 :failure/message string
 :failure/context map
 :failure/children [Failure ...]}
```

Alternative mismatch uses a `:typed/no-alternative` parent with one child tree
per declared branch. `failure-seq` walks leaves depth-first in declaration
order and `failure-count` counts leaves only. A missing map input retains
`nil` as its actual value and adds `{:present? false}` to Failure context.

`validator` and `explainer` compile pure reusable closures. `check` returns one
native `Result<boolean>`: a normal mismatch is success/false with portable
Failure data in Result context, while a checker crash is a Result error.

`std.typed` is the curated public facade over schema, registry, explanation,
checking, and inference operations. Executable predicates remain local runtime
values and are never stored in explanation or Result context.
