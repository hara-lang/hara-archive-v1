# Runtime evaluator boundary

Each Runtime owns exactly one internal Evaluator. Evaluator is not a native
type and has no public HAL surface.

The Runtime remains responsible for namespace selection, NamespaceRegistry,
macros, protocols, modules, packages, capability providers, and transactional
rollback. It installs that context around an evaluation and delegates only
execution state to Evaluator.

The Evaluator boundary consists of:

- source/form evaluation;
- lexical environment snapshot and restoration;
- tree evaluation for traced execution;
- CPS fiber creation and completed-environment adoption.

Evaluator does not receive Kernel, Session, mount, authority, package catalog,
provider registry, or NamespaceRegistry ownership. Java lexical frames remain
Truffle execution state; Rust lexical bindings are stored directly by its
Evaluator. Namespace switching for `Runtime/eval-in` stays a Runtime operation.

Non-loading symbol resolution is a Base primitive (`Base/resolve`). It reads
only already materialized registry entries and never selects a namespace, loads
a package, or enters the evaluator. Dynamic form evaluation stays on
`Runtime/eval`; there is intentionally no `Base/eval` native method.
