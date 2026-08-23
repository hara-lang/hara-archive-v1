# std.work task and provider redirection

Tracking: #392, #490, #491, #492, #493, #494

## Decision

`std.work` is the complete execution, template, event, result, summary and
baseline reporting abstraction. It is not only a minimal workflow kernel.

The complete accepted `std.task` behavior must be expressible through
`work.flow.task`, which compiles into ordinary Work nodes and executes through
the canonical evaluator and executor capabilities.

## Baseline

The core distribution retains:

```text
inline executor
memory store
baseline clocks
committed event observers
structured report documents
plain and terminal reporters
task template and conformance
```

## Extensions

Heavyweight and externally authorised providers move to separately installable
artifacts while preserving namespaces for one migration release:

```text
SQLite
PostgreSQL
Ignatius receipts/signing
remote workers and queues
containers/cloud
external telemetry
```

Provider descriptors declare a versioned API and one of `:baseline` or
`:extension` tiers.

## Protocol laws

```text
IWork         describes immutable work structure and identity
IWorkHost     owns live execution and run lifecycle
IWorkRun      exposes status, results, cancellation and events
```

Reporting is an observer composition; no fourth public protocol is introduced.
Provider contracts are an explicit SPI rather than part of the work algebra.

## Reporting laws

```text
event -> report reducer -> report document -> renderer -> injected sink
```

Portable report models, profiles, sections, tables, summaries, plain rendering
and terminal rendering remain baseline. The host owns stdout/stderr, TTY,
terminal width, colour capability and destination authority.

Live committed events and stored history use the same reducer. Reporting never
changes returned data. Parallel execution may complete out of order while the
default report remains deterministic by input order.

## Task template

The template surface covers:

```text
defaults and identity
input/environment/lookup construction
list and selectors
item pre/run/post/output/display
execution ordering/random/parallel/fail-fast policy
warnings/errors/results
columns, formatting and ordering
summary aggregation/finalisation/timings/annotations
titles, labels and section profiles
return selection and map/vector packaging
```

Local definitions may contain functions. Portable definitions reference trusted
operations through the recipe registry. Both compile to the same work graph.

## First slice

This branch establishes:

- provider API/tier descriptors;
- explicit baseline classification for the inline executor and inferred
  classification for existing providers;
- the portable report document/profile contract;
- a working local `work.flow.task` compiler built from existing Work
  and command operators;
- committed task-stage events;
- structured warning/error/result/summary/report output;
- a pinned Foundation task parity ledger that records remaining gaps rather
  than declaring premature parity.

## Next slices

1. Split mandatory baseline store/executor operations from optional outbox,
   claims, leases, scheduling and receipt capabilities.
2. Add live/history report reduction plus plain and terminal renderers.
3. Restore defaults, main arity adapters, construct environment/lookup,
   additional arguments and complete item/result transforms.
4. Restore deterministic parallel reporting, complete timings, annotations and
   map/vector packaging.
5. Add portable operation-reference task definitions.
6. Migrate `code.test`, `code.manage`, `lang.seedgen`, language management and
   `code.deploy` to the shared template/report system.
7. Extract PostgreSQL, SQLite, Ignatius and other extension providers.
8. Remove legacy task compatibility only after the accepted Foundation corpus
   and first-party tooling parity gates pass.
