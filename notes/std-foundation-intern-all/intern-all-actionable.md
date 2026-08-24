# `intern-all` actionable migration set

Audit baseline: `69cd5b7c444b6bfd9c73965b651ae54bd091ac30` (`main`, 2026-08-19).

This file is the condensed implementation list from the repository-wide façade audit. The full reasoning is in [`intern-all-audit.md`](./intern-all-audit.md), portable script publication is covered in [`intern-all-script-focus.md`](./intern-all-script-focus.md), and the namespace-role migration is surveyed in [`porcelain-namespace-survey.md`](./porcelain-namespace-survey.md).

## Governing rules

- `intern-all` publishes every public Var from a deliberately coherent owner namespace.
- `intern-in` publishes selected or renamed Vars.
- Both macros are already part of the ordinary Foundation surface. Do **not** add `[std.foundation :as f]`; use `intern-all` and `intern-in` unqualified.
- Publication remains explicit in source. There is no `:export true` namespace option.
- `:access true` acknowledges intentional use of an internal namespace; it does not publish it.
- A strict `:facade` namespace contains only top-level `intern-all` and `intern-in` forms after its `ns` declaration.
- Removing `defn-` from an owner selected by `intern-all` generally requires moving non-exported helpers into an internal `.util` or domain-specific namespace first.

## First pilot: `std.format`

Target shape:

```hara
(ns std.format
  (:config {:role :facade})
  (:require [std.format.common]
            [std.format.table]
            [std.format.report]
            [std.format.render]
            [std.format.terminal]))

(intern-all std.format.common
            std.format.table
            std.format.render
            std.format.terminal)

(intern-in std.format.report/report-lines)
```

Before this can be correct:

1. Move the current root `report` adapter into `std.format.render`.
2. Move `std.format.table` helper functions such as `record-role`, column normalisation, width calculation and `table-text` into `std.format.table.util`.
3. Move report-only helpers into `std.format.report.util`.
4. Move terminal constants and `render-line` into `std.format.terminal.util`.
5. Mark all owner and utility namespaces `:role :internal`.
6. Convert their former private definitions into ordinary public definitions.
7. Add direct tests for the utility owners and a public-surface test for `std.format`.

This preserves the existing supported API while making every implementation function directly testable.

## Second pilot: `workspace`

Target shape:

```hara
(ns workspace
  (:config {:role :facade})
  (:require [workspace.core]
            [workspace.model]))

(intern-all workspace.core)

(intern-in workspace.model/area
           workspace.model/component
           workspace.model/component-view
           workspace.model/find-area
           workspace.model/component-contract)
```

Move `reject`, `select-area`, and `route-area-event` from `workspace.core` into `workspace.transition`. Keep only `create`, `dispatch`, `view`, and `result` in the coherent export owner. Mark `workspace.core`, `workspace.transition`, and `workspace.model` internal.

## Clear curated facades

The following roots should become `:facade` after their local implementation has moved into internal owners:

| Root | Required preparation | Publication style |
|---|---|---|
| `std.typed` | Mark schema, registry, explain and infer owners internal. | Curated and renamed `intern-in`. |
| `std.config` | Move session operations to `std.config.session`. | Selected `intern-in`. |
| `std.block` | Use `std.block.navigate` for navigation and move source helpers to `std.block.source`. | Curated and renamed `intern-in`. |
| `code.vm` | Mark model/source/interpreter/HALC/bytecode/conformance owners internal. | Renamed `intern-in` families. |
| `code.test` | Move root macros and compatibility adapters to `code.test.api`. | `intern-all code.test.api` plus selected `intern-in`. |
| `tool.lint` | Move Result-boundary entrypoints to `tool.lint.api`. | `intern-all`/`intern-in` from API and report owners. |
| `db.postgres` | Move lifecycle code to `db.postgres.lifecycle`; extract connection helpers. | Selected connection and lifecycle publication. |
| `lang.runtime.basic` | Move registry installation into a bootstrap owner. | Curated `intern-in`. |

## Major decompositions

Handle these after the pilot establishes role, access, lint and publication semantics:

- `std.fs`: split I/O, copy, delete and temporary-resource owners; keep path/walk standard if directly supported.
- `std.lib.time`: split model, calendar, arithmetic, formatting and parsing owners.
- `work.base`: split algebra, graph, host API, runtime API and definition-macro owners.
- `work.agent`: split model, host, transitions, execution loop and public API.
- `lang.core`: split bootstrap, pointer/fragment registration, script API and grammar-derived macro generation before making the root porcelain.
- `std.foundation`: migrate last because new internal children must be embedded and loaded identically by Rust and Java.

## Standard namespaces needing helper extraction

These should normally remain `:standard` initially while private helpers move into internal children:

- `std.codec.hex`, `std.codec.base64`, `std.codec.url`, `std.codec.form`;
- `std.text.diff`, `std.logic.datalog`;
- `std.lib.collection`, `std.lib.component`;
- `std.fs.path`, `std.fs.walk` when kept public;
- `tool.sh`, `tool.sh.git`, `tool.sh.docker`, `tool.sh.tmux`;
- `tool.package`, `tool.vm`, `tool.project`, `tool.runtime`, `tool.inrepl`;
- `code.deploy` and other roots retained as direct implementation APIs;
- `db.ledger.chain` and direct public database utilities.

Prefer a meaningful domain owner over `.util` whenever the helpers form a real subsystem.

## Mostly mechanical internalisation

Most child implementation families can be converted in place:

- `lang.common.*`, `lang.model.v1.*`, `lang.runtime.*`, and `lang.core.*` implementation children;
- `code.vm.*` and `code.translate.clojure`;
- `db.node.*` implementation children;
- `work.flow.make.*`, `work.flow.task.report`, `work.base.host`, and `work.base.execution.graph`;
- `tool.cli.report`, `tool.cli.verify`, and `tool.metaspec.*`;
- `std.typed.*`, `std.block.*`, and `std.substrate.result` implementation children.

For each namespace:

1. add `(:config {:role :internal})`;
2. replace top-level `defn-`, `defmacro-`, and `^:private` definitions with ordinary public definitions;
3. add direct tests;
4. do not publish the complete namespace unless its whole surface has been deliberately designed as an export owner.

## Platform work required first

1. Add a canonical portable namespace-declaration parser returning role and require/access data.
2. Add `:role` parsing to Java and Rust; default to `:standard`.
3. Add `[namespace :access true]` support and retain access edges.
4. Attach `:project/id` and role metadata to registered resources and runtime namespace descriptors.
5. Add porcelain lint findings for private top-level Vars, unacknowledged internal access, non-publication facade forms and publication collisions.
6. Strengthen interning with deterministic ordering, collision preflight, provenance and reload reconciliation.
7. Add package API and internal-access manifests.

## Validation

For every facade conversion:

- compare sorted target `ns-publics` before and after;
- compare Var metadata, including macros, schemas and dynamic Vars;
- test qualified, referred and used access where supported;
- verify no helper namespace is accidentally interned;
- test intentional cross-project internal access with and without `:access true`;
- run Java and Rust namespace-loader parity;
- regenerate script and migration inventories where grammar-owned namespaces are involved.

## Delivery order

1. Namespace role/access specification and parser parity.
2. Warning-mode lint rules and syntax-aware inventory.
3. `std.format` pilot.
4. `workspace` pilot.
5. Mechanical internal leaf conversion.
6. Standard namespace helper extraction.
7. Clear facade conversions.
8. Large root decompositions.
9. Package/runtime enforcement.
10. Enable the no-private-top-level policy for Hara itself.
