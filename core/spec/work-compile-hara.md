# Source-owned Work Hara lowering

Status: bounded implementation contract for #808. The direct evaluator remains
the semantic authority; this backend exposes the same Work value as inspectable
ordinary Hara data and reconstructs it through `work.base/definition`.

## API

```hara
(work.compile.hara/ir work)
(work.compile.hara/form work)
(work.compile.hara/form work {:ir/version 1})
```

`ir` returns a versioned map:

```text
{:ir/version 1
 :ir/work {:ir/kind :work
           :ir/value <ordinary Work spec>}}
```

Nested Work values are represented by another `:ir/kind :work` node. Function
values are represented explicitly as `{:ir/kind :closure :ir/value closure}`.
Map traversal is ordered by the string form of its keys so repeated lowering
within one process assigns closure references in a stable order.

Set traversal uses the same ordering rule. Source-owned `GraphInput`,
`GraphNode`, and `GraphDeps` values have explicit `:graph-input`, `:graph-node`,
and `:graph-deps` IR nodes, so graph structure is inspectable rather than an
opaque host object.

`form` returns an ordinary Hara list whose head is
`work.base/definition`. Nested Work values are nested forms and closure values
are source-owned `work.compile.hara/resolve-closure` references. The form does
not contain provider handles, executor/store objects, or a second evaluator.
Evaluating it reconstructs the original Work specification; execution still
enters the canonical evaluator.

Graph values are reconstructed with the source-owned `work.base/GraphInput`,
`work.base/GraphNode`, and `work.base/GraphDeps` constructors.

## Closure boundary

Closure references are process-local executable values, not durable data. They
are deliberately explicit in the IR and are registered only while producing an
executable form. `reset-closures` is the lifecycle boundary for the registry;
callers must evaluate generated forms before resetting it. Durable Work must
continue to reconstruct pure continuations from source/provider definitions and
must never persist a closure registry token as a checkpoint result.

This boundary supports captured lexical state without pretending that arbitrary
runtime closures can be serialized. It also makes analysis able to distinguish
ordinary Work structure from executable closure leaves.

## `bind` equivalence

Because lowering preserves the complete Work spec recursively, `:bind` retains
the direct evaluator law:

```text
execute(source, input)
execute(pure-continuation, source-result)
execute(produced-work, source-result)
```

The generated form adds no bind handler, host submission, run identity, or
checkpoint model. Source, continuation, and produced children therefore retain
the existing `:bind/:source`, `:bind/:continuation`, and `:bind/:produced`
lineage, dynamic-depth checks, cancellation behavior, and managed replay.

## Evidence

The permanent focused proof is:

```shell
./scripts/runtime/run-lib-tests \
  core/lib/test/work/compile/hara_test.hal
```

The test covers ordinary Work validation, captured closure inspection, versioned
IR, closure reset/lookup, generated-form reconstruction, and direct-vs-form
`bind` result equivalence. The native `manage scaffold` command described by
`AGENTS.md` is unavailable in the checked-in CLI; the corresponding test path
was created manually and remains paired with this source namespace.

Cross-runtime direct-callable and namespace-role evidence remains owned by
#838 and #844 respectively. This backend consumes those contracts but does not
silently claim their outstanding JVM Foundation-origin, access-policy, facade,
or browser/Wasm gates.
