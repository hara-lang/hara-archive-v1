# Work Capability ABI

Status: implementation contract for the `work.*` algebra migration tracked by
#803, #805, and #806. Runtime-produced Work continuation is tracked by #880.

## Purpose

The Work kernel separates immutable computation descriptions from execution,
persistence, and live process ownership.

```text
IWork
  describes Work
       |
       v
Work evaluator or compiler
       |
       +--------------------+
       |                    |
       v                    v
IWorkExecutor            IWorkStore
execute one leaf         query and journal state
       |                    |
       +----------+---------+
                  v
            Runtime value
                  |
                  v
      optional IWorkHost / IWorkRun
```

The evaluator owns the structural meaning of Work operations. Executors never
interpret Work composition, and stores never execute Work.

## Native protocol family

The public native Work family is:

```text
IWork
IWorkExecutor
IWorkStore
IWorkRef
IWorkRun
IWorkHost
```

No separate runtime or machine protocol is defined. Runtime configuration is
ordinary Hara data. Runtime-produced Work under `:bind` does not add another
protocol.

## IWorkExecutor

```hara
(defprotocol IWorkExecutor
  (work-execute [executor request]))
```

`work-execute` performs one leaf request. It may return a direct value or an
`IPromise`.

A leaf request is a map. The stable vocabulary is expected to include:

```hara
{:work/request :execute-leaf
 :run/id optional-run-id
 :work/root work-root
 :work/boundary :step
 :node/id node-id
 :node/path node-path
 :item/id optional-item-id
 :work/target target
 :work/input input
 :work/context context
 :work/attempt attempt
 :work/deadline optional-deadline}
```

The target profiles are explicit:

```hara
{:target/type :local :target/fn local-callable}
{:target/type :qualified :target/name portable-name}
{:target/type :pinned
 :target/name portable-name
 :target/version version
 :target/digest content-digest}
```

Only the local profile carries a callable. Qualified targets resolve by name;
pinned targets resolve by the exact `[name version digest]` registry key, with
no unpinned fallback.

`work.eval/run` is the store-free reference path. It owns every structural Work
operation and invokes `IWorkExecutor` only for `:step` leaves. `:pure` remains
an evaluator-local computation and is never sent to an executor. The inline
executor accepts explicit local closure targets and named targets from its
immutable target map.

`:bind` is also evaluator-owned. It executes source Work, evaluates a pure Work
continuation that returns another `IWork`, then recursively executes the
produced Work in the same Runtime and run/frame lineage. Neither the bind node
nor its continuation is sent to `IWorkExecutor`; only ordinary `:step` leaves
inside the source or produced subtree cross the executor ABI. A bind must never
submit produced Work through `IWorkHost` or start a nested run.

The produced subtree inherits the canonical leaf envelope. Its step requests
therefore preserve the current run/root identity, deterministic node path,
item identity, user Work context, deadline, retry policy, and executor/store
authority. Runtime production cannot be used to escape or replace those
capabilities.

Legacy executor provider maps are rejected. Concrete executors must implement
`IWorkExecutor` and receive only the canonical leaf request; they never receive
the enclosing structural Work tree.

`IWorkExecutor` does not extend `IComponent`. A concrete process pool, remote
worker, or sandbox executor may separately implement lifecycle protocols.

## Runtime-produced Work replay

Managed `:bind` execution does not persist an executable Work value as a
checkpoint result. Instead:

- effectful source steps checkpoint normally;
- the pure continuation may be recomputed during resume;
- it reconstructs the same stable produced Work subtree;
- completed produced steps replay through their existing checkpoint paths;
- only unfinished effects execute again.

Durable runtime-produced Work therefore requires an explicit stable `:id`.
Dynamic production is bounded by the bind depth policy, and produced descendants
inherit the strictest active maximum. Cancellation and deadlines are checked by
the ordinary recursive evaluator before every child boundary.

These are evaluator/store laws, not new methods on `IWorkExecutor`,
`IWorkStore`, or `IWorkHost`.

## IWorkStore

```hara
(defprotocol IWorkStore
  (work-query [store query])
  (work-transact [store transition]))
```

`work-query` performs a typed read. The baseline managed-execution query family
covers:

```text
run load and list
committed event history
checkpoint load and list
```

A query is an immutable map whose discriminator is supplied by
`:work/query`. The baseline query vocabulary is exactly:

```hara
{:work/query :run/load :run/id id}
{:work/query :run/list :work/where where}
{:work/query :event/list :run/id id}
{:work/query :checkpoint/load :checkpoint/key key}
{:work/query :checkpoint/list :run/id id}
```

Existing provider operation maps are wrapped by `StoreAdapter`; managed
evaluation itself dispatches only through `IWorkStore`.

`work-transact` applies one revision-fenced journal transition. A transition
may atomically contain:

```text
run creation or updates
checkpoint commits
committed events
```

Run creation uses `:transition/create-run` with
`:transition/expected-revision :absent`. It is committed through
`work-transact`, not through an independent write method.

The store must preserve these laws:

- a failed expected-revision check performs no partial writes;
- a checkpoint identity cannot be committed with a different value;
- a checkpoint and its corresponding completion event commit atomically;
- observers see only events returned by the store as committed;
- committed event order is stable for one run.

Transactional outbox, claims, leases, delayed scheduling, receipt publication,
and distributed fencing are optional capability suites rather than mandatory
`IWorkStore` methods.

The baseline store capability set is runs, transactions, events, and
checkpoints. Outbox operations remain accessible only through the explicit
`:outbox` extension capability and are omitted entirely by baseline-only
stores.

`IWorkStore` does not extend `IComponent`. A concrete database client may
separately implement lifecycle protocols.

## Runtime

Runtime is a reusable immutable map or record that assembles capabilities and
policy:

```hara
{:work/executor executor
 :work/store store-or-nil
 :work/registry registry-or-nil
 :work/policy policy-map
 :work/hooks hooks-map}
```

It has no native protocol and owns no thread, process, stream, run identity, or
cancellation lifecycle.

Bare evaluation requires an executor and may omit the store. Managed execution
adds store-backed step replay and journal transitions. The same Runtime value
may be reused across many live runs.

## Host boundary

`IWorkHost`, `IWorkRun`, and `IWorkRef` remain orthogonal to the evaluator,
executor, and store ABIs.

A host owns live concerns:

```text
run identity
status and asynchronous result
cancellation and deadlines
structured child runs
live event streams
finalisation
```

The native host accepts an execution adapter. It does not define the meaning of
Work operations and does not become a store.

## Compatibility phase

During the migration:

- existing executor and store provider descriptors remain accepted;
- existing Work maps and operation keywords retain their shapes;
- existing run, event, checkpoint, and PostgreSQL record shapes remain stable;
- adapters bridge provider operation maps to the native capability protocols;
- Runtime constructors may continue returning the existing struct while also
  accepting the canonical map contract.
