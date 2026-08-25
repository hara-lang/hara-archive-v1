# Repository-wide `std.foundation/intern-all` audit

Audit baseline: `69cd5b7c444b6bfd9c73965b651ae54bd091ac30` (`main`, 2026-08-19).

This note preserves the repository-wide audit that was originally presented as transient chat preview artifacts. It records the decision rule, confirmed candidates, exclusions, and validation requirements for replacing hand-written façade definitions with `std.foundation/intern-all` or `std.foundation/intern-in`.

The condensed implementation list is in [`intern-all-actionable.md`](./intern-all-actionable.md). Script publication and the migration ledger are analysed separately in [`intern-all-script-focus.md`](./intern-all-script-focus.md). A machine-readable inventory is in [`intern-all-candidates.tsv`](./intern-all-candidates.tsv).

## Executive result

The audit found:

1. **Two immediate whole-surface migrations**:
   - the `std.format.common`, `std.format.table`, and `std.format.terminal` portions of `std.format`;
   - the `workspace.core` portion of `workspace`.
2. **One existing reference implementation**:
   - `postgres.core` already uses `std.foundation/intern-all` for complete public owner namespaces and `intern-in` for selected exports.
3. **A larger set of selective façade aliases** that should converge on `std.foundation/intern-in`, not unrestricted `intern-all`.
4. **A substantial set of semantic wrappers, generated definitions, bootstrap code, and introspection utilities** that should remain explicit.

The central finding is that `intern-all` is a **namespace publication primitive**. It is not a textual shortcut for every expression shaped like `(def x owner/x)` or `(defn x [...] (owner/x ...))`.

## What `intern-all` guarantees

The implementation in `std.foundation` enumerates `(keys (ns-publics source))` and interns each source Var into the current namespace with the same unqualified symbol. Consequently, a safe conversion requires all of the following:

- every public Var in the source namespace is intended to be public in the target;
- target and source names are identical;
- there are no collisions between imported source namespaces or local target definitions;
- source Var identity and metadata are the intended target contract;
- source loading before expansion is deterministic;
- expanding the source public surface later is intended to expand the target façade automatically.

When any of these are false, use `intern-in` or keep an explicit definition.

## Classification model

### `apply-now`

The target publishes an owner namespace's complete public surface under the same names, with no semantic adaptation.

### `existing-correct`

The target already uses `intern-all` at an appropriate whole-owner boundary.

### `selective-intern-in`

The target republishes only selected Vars, uses renamed public names, or combines several owners whose complete public surfaces would be too broad or collide.

### `keep-explicit`

The definition adapts behaviour or is itself the stable compatibility boundary. This includes argument/return transformation, exception shaping, asynchronous behaviour, lifecycle management, native capability boundaries, macro expansion, or deliberate metadata changes.

### `generator-owned`

The definitions are produced from a grammar, migration ledger, or target-language generation path. Change the generator or governing ledger rather than hand-editing generated output.

### `bootstrap/introspection`

The use of `ns-publics` or `intern-var` is part of environment bootstrapping, analysis, parity checking, or source introspection rather than façade publication.

## Confirmed findings

| Classification | Target | Source owner(s) | Finding |
|---|---|---|---|
| `apply-now` | `core/lib/src/std/format.hal` | `std.format.common` | All four public Vars are already re-exported unchanged: `line`, `value-text`, `truncate`, `pad`. |
| `apply-now` | `core/lib/src/std/format.hal` | `std.format.table` | Its sole public Var, `table-lines`, is re-exported unchanged. |
| `apply-now` | `core/lib/src/std/format.hal` | `std.format.terminal` | Its complete public surface, `render-lines`, `render`, and `emit-lines!`, is re-exported unchanged. |
| `selective-intern-in` | `core/lib/src/std/format.hal` | `std.format.report` | Publish only `report-lines`; keep the local `report` adapter, which renders through the terminal layer. |
| `apply-now` | `core/lib/src/workspace.hal` | `workspace.core` | All four public Vars—`create`, `view`, `result`, `dispatch`—are re-exported unchanged. |
| `selective-intern-in` | `core/lib/src/workspace.hal` | `workspace.model` | The façade selects `area`, `component`, `component-view`, `find-area`, and `component-contract`, while omitting other public model helpers and constants. |
| `existing-correct` | `core/lib/src-lang/postgres/core.hal` | `postgres.core.builtin`, `postgres.core.addon` | Already uses `f/intern-all` for complete public owner surfaces and `f/intern-in` for selected cross-owner exports. |
| `selective-intern-in` | `core/lib/src/std/typed.hal` | schema, registry, explain, infer owners | This is explicitly a curated façade. Its owners expose lower-level helpers and constants that are not part of the public `std.typed` contract. |
| `selective-intern-in` | `core/lib/src/std/block.hal` | base, construct, parse, type, value, layout owners | The namespace mixes same-name aliases with deliberate renames such as `type <- block-type`, `string <- block-string`, and `layout <- layout-main`, plus local zipper/source helpers. |
| `selective-intern-in` | `core/lib/src/tool/lint.hal` | analyze, flow, report owners | Direct aliases can use `intern-in`, but lint entrypoints wrap results in native `Result` values and must remain explicit. |
| `selective-intern-in` | `core/lib/src/work/base/runtime.hal` | model, memory, frame, coordinator, receipt owners | The runtime surface is curated and retains compatibility constructors; whole-owner publication would expose implementation helpers. |
| `selective-intern-in` | `core/lib/src/work/base.hal` | `work.base.runtime` | Only the selected runtime façade block is eligible. The namespace itself owns the Work algebra, graph types, host operations, and lifecycle API. |
| `selective-intern-in` | `core/lib/src/work/flow/task.hal` | `work.flow.task.engine` | The public task façade selects constructors and profile operations while hiding compilation/execution internals. |
| `selective-intern-in` | `core/lib/src/work/flow/make.hal` | `work.flow.make.host` | `host?` is a selected singleton export. The remaining public functions are local orchestration and trigger adapters. |
| `selective-intern-in` | `core/lib/src/lang/runtime/basic.hal` | basic, oneshot, verify owners | Each owner has additional public setup and result helpers. The façade also installs runtime registry coordinates as a load-time effect. |
| `selective-intern-in` | `core/lib/src/std/config.hal` | global and resolve owners | The public surface is curated and includes local session-state wrappers; whole-owner import would expose resolver internals. |
| `selective-intern-in` | `core/lib/src/std/block/heal.hal` | `std.block.heal.core` | `heal` is a renamed publication of `heal-content`; rainbow rendering functions are local wrappers. |
| `keep-explicit` | `core/lib/src/code/test.hal` | checker, compile, runtime, work, CLI owners | The façade includes macros, renamed comparison functions, stable compatibility spellings, and collision-prone symbols such as `run`, `all`, and `check`. |
| `keep-explicit` | `core/lib/src/code/vm.hal` | model, source, interpreter, HALC, bytecode, conformance owners | Prefixes such as `interpreter-*`, `halc-*`, and `bytecode-*` deliberately avoid collisions and preserve one coherent façade. |
| `keep-explicit` | `core/lib/src/db/postgres.hal` | connection and provider owners | The namespace combines selected aliases with connection lifecycle, temporary database management, Docker integration, and component behaviour. |
| `keep-explicit` | `core/lib/src/lang/core.hal` | book, grammar, preprocess, registry, script, runtime owners | The namespace is a bootstrap and generation boundary, not a passive façade. It owns pointer registration, runtime selection, language installation, and grammar-derived macro publication. |
| `keep-explicit` | `core/lib/src/std/substrate.hal` | substrate internal modules | It owns `SubstrateNode`, protocol implementations, transport lifecycle, request routing, compatibility constructors, and result adaptation. |
| `keep-explicit` | `core/lib/src/code/translate.hal` | translation rule and Clojure owners | It combines selected aliases with adapted namespace-shape functions and a project-level Work graph. |
| `keep-explicit` | `core/lib/src/code/manage.hal` | management unit and task owners | Many public values are constructed workflow/task products rather than aliases of source Vars. |
| `keep-explicit` | `core/lib/src/std/lib/kernel.hal` | `std.lib.kernel` | The retired `std.sandbox` path is not restored. Sandbox lifecycle operations and their native wrappers remain owned directly by `std.lib.kernel`. |
| `generator-owned` | `core/lib/src-lang/xt/substrate.hal` | XTalk grammar modules | `def.xt` declarations and schemas belong to the target-language generation path. They must not be replaced by host-level `intern-all`. |
| `generator-owned` | `core/lib/src/code/migrate/script.hal` | `tahto.core.script*` migration family | This is the governing ledger for script migration and publication. Change its dispositions/generators rather than editing generated macro families independently. |
| `bootstrap/introspection` | `core/lib/src/std/foundation/bootstrap.hal` | runtime namespace bootstrap | `ns-publics` is used to construct and validate bootstrap namespace surfaces, not to hand-publish an application façade. |
| `bootstrap/introspection` | `core/lib/src/tool/lint/profile.hal` | analysed namespaces | Public Var enumeration contributes to lint profiles and symbol discovery; it is not a candidate for `intern-all`. |
| `bootstrap/introspection` | `core/lib/src/lang/common/grammar.hal` | grammar namespaces | Namespace inspection supports grammar construction and reserved-symbol discovery. Preserve it as analysis logic. |
| `bootstrap/introspection` | `scripts/runtime/foundation_parity.py` | source and target inventories | The script parses `intern-in`/`intern-all` to calculate parity surfaces. It validates publication; it is not itself a publication site. |

## Direct migration details

### `std.format`

The current façade contains eight direct aliases from three complete owner surfaces. They should become:

```hara
(f/intern-all std.format.common
              std.format.table
              std.format.terminal)
```

The `std.format.report` owner is different. Importing its entire public surface would introduce a `report` Var that collides with the façade's local `report` adapter. Use:

```hara
(f/intern-in report-format/report-lines)
```

and keep the local adapter explicit.

### `workspace`

The complete `workspace.core` public surface should become:

```hara
(f/intern-all workspace.core)
```

The selected `workspace.model` surface should become:

```hara
(f/intern-in model/area
             model/component
             model/component-view
             model/find-area
             model/component-contract)
```

This preserves the existing target surface without exposing `normalize`, `default-area-id`, `workspace-type`, or `workspace-version`.

## Why direct aliases are still not automatically safe

Even `(def target owner/target)` can be intentionally different from `intern-all`:

- it creates a target-owned Var rather than publishing the source Var;
- target metadata can differ from source metadata;
- redefining the target need not affect the source;
- source namespace growth does not automatically grow the target API;
- several source owners can expose the same public symbol without an immediate collision;
- load order can differ because `intern-all` needs the source namespace's public surface at macro expansion time.

These differences are useful in compatibility façades. Migration should be deliberate rather than purely syntactic.

## Metadata and identity policy

A successful conversion intentionally makes the target name refer to the source Var publication contract. Before changing a façade, compare at least:

- symbol and namespace identity;
- `:doc`;
- `:arglists`;
- `:schema` and related typed metadata;
- `:macro`;
- `:dynamic`;
- `:private`;
- `:deprecated`;
- `:added`;
- any compiler or inline metadata.

If target-owned metadata is part of the public contract, keep the target definition or add an explicit migration mechanism rather than silently dropping it.

## Collision policy

`intern-all` must not be used when two imported namespaces publish the same unqualified symbol unless collision behaviour is explicitly defined and tested. Common collision families in this repository include:

- `run`, `start`, `stop`, `status`, `result`, and `events`;
- `create`, `compile`, `validate`, and `resolve`;
- `all`, `filter`, `check`, and `report`;
- constants named `+schema+`, `+version+`, or `+default-*+`.

A façade that prefixes or renames these symbols is expressing API design, not accidental verbosity.

## Generated and migration-led publication

For `lang.core`, `lang.core.script`, and target-language namespaces, publication is intertwined with:

- grammar-derived definition families;
- `def.*`, `def$.*`, `defmacro.*`, and `!.*` generation;
- target-language pointers;
- module and runtime registration;
- loader parity across JVM and Rust;
- the `code.migrate.script` ledger.

The correct change point is the governing generator or ledger entry. Hand-converting generated definitions would produce drift and would not survive deterministic regeneration.

## Required verification

### Surface equality

Capture before and after values equivalent to:

```hara
(vec (sort-by str (keys (ns-publics 'target.namespace))))
```

The public symbol set must be identical unless an API expansion is separately reviewed and recorded.

### Var metadata

For each moved symbol, compare the selected metadata fields described above. Macro and dynamic Var metadata are mandatory, not optional.

### Runtime profiles

Run publication/load tests on both supported native loader paths. A macro that works in the JVM interpreter but is absent from the Rust bootstrap surface is not migrated successfully.

### Generated families

Validate representative generated entries, including at least one `def.xt` and one `def.pg` path, plus deterministic regeneration of the complete family.

### Migration ledger

Require:

- no missing source references;
- no duplicate source references;
- no stale target references;
- no unexplained target-only entries;
- explicit dispositions for overlapping `lang.core` and `lang.core.script` compatibility paths.

### Behaviour

Exercise qualified use, `:refer`, and `:use` where supported. Confirm that later local definitions do not silently replace imported Vars and that source namespace growth cannot accidentally broaden a curated façade.

## Recommended implementation sequence

1. Convert `std.format` and add a focused surface/metadata regression test.
2. Convert `workspace` using one `intern-all` and one `intern-in` form.
3. Consolidate straightforward same-name alias blocks into `intern-in` without changing their target surfaces.
4. Split owner namespaces only where there is a deliberate requirement for a whole-surface façade.
5. Handle portable script publication through `code.migrate.script` and the grammar generators.
6. Add a lint rule that detects long same-name alias blocks and reports either:
   - `intern-all` when the source public surface is exact;
   - `intern-in` when the target is curated;
   - an explicit disposition when a wrapper is semantic.

## Conclusion

The repository should use `intern-all` more often, but only at clear owner-to-façade boundaries. The immediate conversions are intentionally small because unrestricted bulk publication is an API commitment: every current and future public Var in the owner becomes part of the façade. The wider cleanup should primarily use `intern-in`, while bootstrap, native adapters, generated language families, and semantic compatibility wrappers remain explicit.
