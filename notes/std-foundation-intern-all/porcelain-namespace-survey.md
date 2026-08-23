# Porcelain namespace model: Hara migration survey

Survey baseline: `69cd5b7c444b6bfd9c73965b651ae54bd091ac30` (`main`, 2026-08-19).

This note formalises the preferred Hara namespace structure and surveys the repository changes needed to remove top-level private definitions. It supersedes the earlier `:export true` proposal: publication remains explicit through `intern-all` and `intern-in`; `:access true` acknowledges intentional use of an internal namespace.

## Agreed namespace contract

### Roles

| Role | Default | Definitions | Publication forms | External contract |
|---|---:|---|---|---|
| `:standard` | yes | ordinary `def`, `defn`, `defmacro`, protocols and types | `intern-all` and `intern-in` allowed | supported namespace API |
| `:internal` | no | ordinary public definitions; no top-level private Vars | `intern-all` and `intern-in` allowed, but the resulting namespace remains internal | guarded implementation API |
| `:facade` | no | none | only top-level `intern-all` and `intern-in` forms | supported porcelain API |

Omitting `:role` means `:standard`. Internal and facade namespaces declare it explicitly:

```hara
(ns example.codec.parse
  (:config {:role :internal}))

(ns example.codec
  (:config {:role :facade})
  (:require [example.codec.model]
            [example.codec.parse]
            [example.codec.encode]))

(intern-all example.codec.model)
(intern-in example.codec.parse/parse
           example.codec.encode/encode)
```

`std.foundation` does not need to be required or aliased. `intern-all` and `intern-in` are part of the ordinary Foundation surface.

### Intentional internal access

```hara
(ns another.project.experiment
  (:require [example.codec.parse :as parse :access true]))
```

`:access true` means “this dependency knowingly uses an internal namespace.” It does not publish anything and does not make the internal namespace standard.

The intended enforcement is:

- same-project implementation and tests may require internal namespaces normally;
- cross-project access to an internal namespace requires `:access true`;
- `:access true` may also be written inside a project as documentation of an exceptional dependency;
- re-exporting a cross-project internal Var requires both `:access true` on the require and an explicit `intern-all` or `intern-in` publication form;
- dependency manifests record intentional internal access and internal re-export separately.

This is an API-stability boundary, not a security boundary.

## Why `.util` extraction is sometimes necessary

Removing `defn-` by merely changing it to `defn` is safe only when the namespace is not published wholesale.

If a facade contains:

```hara
(intern-all example.codec.parse)
```

then every public Var in `example.codec.parse` is external. Helpers that should remain implementation details must move to another internal namespace:

```hara
(ns example.codec.parse.util
  (:config {:role :internal}))

(defn scan-token ...)
(defn recover-delimiter ...)

(ns example.codec.parse
  (:config {:role :internal})
  (:require [example.codec.parse.util :as util]))

(defn parse ...)
```

The preferred extraction order is:

1. a meaningful owner such as `.parse`, `.format`, `.calendar`, `.transition`, `.validate` or `.state`;
2. `.util` only for genuinely shared, low-level helpers;
3. never create one package-wide dumping-ground utility namespace.

This produces two kinds of internal namespaces:

- **export owners**, whose complete public surface may be selected with `intern-all`;
- **implementation owners**, which are directly testable but never interned into the porcelain API.

## Repository baseline

The exact textual scan found **146 production `.hal` files containing `(defn-`**:

| Family | Files |
|---|---:|
| `std` | 29 |
| `code` | 10 |
| `lang` | 49 |
| `work` | 15 |
| `tool` | 17 |
| `db` | 25 |
| `workspace` | 1 |

There are also top-level private macros in `lang.core` and `code.migrate.script`, plus `^:private` Vars in the Foundation, formatting, time, and language-rewrite families. The migration rule must therefore cover `defn-`, `defmacro-`, and private metadata consistently.

The dominant migration is mechanical: many current child namespaces are already implementation owners and only need `:role :internal` plus conversion of private top-level definitions to ordinary public definitions. The difficult work is concentrated in mixed root namespaces that combine API assembly with implementation.

## Migration shapes

### A. Internal implementation leaf

Apply when a namespace is not itself an external contract and is not imported wholesale with `intern-all`:

1. add `(:config {:role :internal})`;
2. change top-level `defn-` to `defn`, `defmacro-` to `defmacro`, and private `def` metadata to ordinary `def`;
3. keep helpers in place and add direct tests;
4. update external consumers to use a porcelain or standard namespace, or add `:access true` when the dependency is intentional.

This is the likely treatment for most `lang.common.*`, `lang.model.v1.*`, `lang.runtime.*`, `code.vm.*`, `db.node.*`, `work.flow.make.*`, and similar implementation modules.

### B. Internal export owner

Apply when a facade should use `intern-all` on the namespace:

1. identify the intended complete exported surface;
2. move all other helpers into a sibling internal namespace, preferably with a domain name and otherwise `.util`;
3. make the remaining export-owner definitions ordinary public Vars;
4. publish the owner explicitly with `intern-all`.

`std.format.table`, `std.format.report`, `std.format.terminal`, and `workspace.core` are immediate examples.

### C. Standard public namespace with hidden helpers

Apply when the namespace remains directly supported and is not becoming a facade:

1. leave it `:standard` (usually omit `:role`);
2. move private helpers into `<namespace>.util` or a meaningful internal child;
3. make those moved helpers public and test them directly;
4. preserve the standard namespace public surface.

The codec, shell, collection, component, and several direct utility namespaces fit this shape.

### D. Mixed root becoming a facade

Apply when a root currently owns both implementation and aliases:

1. split local implementation into coherent internal API owners;
2. split their non-exported helpers into internal implementation owners where needed;
3. move root-local macros and adapters into an internal API owner;
4. change the root to `:role :facade`;
5. replace aliases and wrappers with explicit `intern-all` or `intern-in` publication from the new owners.

`std.block`, `code.test`, `work.base`, and eventually `lang.core` require this treatment.

### E. Bootstrap or generated namespace

Foundation bootstrap and grammar-generated language namespaces cannot be moved mechanically. Their role and access metadata must be represented in the generators, bootstrap namespace manifests, and both native loaders. Generated output must remain deterministic.

## High-confidence facade conversions

### `std.format`

Target layout:

```text
std.format                  facade
std.format.common           internal export owner
std.format.table            internal export owner
std.format.table.util       internal implementation owner
std.format.report           internal export owner
std.format.report.util      internal implementation owner
std.format.terminal         internal export owner
std.format.terminal.util    internal implementation owner
std.format.render           internal export owner
```

Move the current root `report` adapter to `std.format.render`. Move table layout helpers such as `record-role`, column normalisation/width calculation and `table-text` to `std.format.table.util`. Move terminal constants and line rendering to `std.format.terminal.util`. The facade can then use `intern-all` for coherent owner surfaces without exposing implementation helpers.

### `workspace`

Target layout:

```text
workspace                   facade
workspace.core              internal export owner
workspace.transition        internal implementation owner
workspace.model             internal owner
```

Move `reject`, `select-area`, and `route-area-event` from `workspace.core` into `workspace.transition`; keep `create`, `dispatch`, `view`, and `result` in the export owner. Then `workspace` can use `intern-all workspace.core` and a curated `intern-in` for model constructors and queries.

### `std.typed`

Mark `std.typed.schema`, `registry`, `explain`, and `infer` internal and make their private helpers ordinary public functions. The root becomes a facade, but remains a curated `intern-in` surface because the child namespaces intentionally contain lower-level operations and several public names are renamed.

### `std.config`

Create `std.config.session` for `get-session`, `swap-session`, and `clear-session`. Mark global, resolve, and session owners internal. The root becomes a facade with selected `intern-in` publication.

### `std.block`

Mark base, construct, parse, type, value, layout, and related children internal. Use `std.block.navigate` for navigation and move source-inspection helpers (`namespace-name`, `grep-source`) to `std.block.source`. The root then becomes a curated facade using `intern-in`, including its deliberate renames.

### `code.vm`

Mark model, source, interpreter, HALC, bytecode, and conformance namespaces internal. Convert their private functions in place unless an owner is later selected for `intern-all`. The root becomes a facade using renamed `intern-in` entries for the `interpreter-*`, `halc-*`, `bytecode-*`, and `conformance-*` families.

### `code.test`

Move all current root-local macro forwarding and metadata-sensitive wrappers into `code.test.api`, an internal export owner. Keep checker, runtime, process, compilation, management, work, artifact, result, and CLI namespaces internal. The root becomes a facade containing only explicit interning forms. This avoids losing the semantic behaviour of `fact`, `capture`, and compatibility macros while still removing definitions from the porcelain namespace.

### `tool.lint`

Move `completed` and the Result-returning public entrypoints into `tool.lint.api`. Mark analyze, flow, report, model, profile, and schema implementation namespaces internal. The root becomes a facade publishing the stable API owner and selected report renderers.

### `db.postgres`

Split the managed lifecycle functions into `db.postgres.lifecycle`. Treat `db.postgres.connection` as an internal export owner after moving its private parsing/validation helpers to `db.postgres.connection.util`. The root can then become a facade over selected connection and lifecycle surfaces.

### `lang.runtime.basic`

Move registry installation side effects to an internal registry/bootstrap owner. Mark type-basic, type-oneshot, type-verify, type-common, transport, and implementation namespaces internal. Once load-time installation is no longer in the assembly file, the root can become a curated facade.

## Major decompositions

### `std.fs`

Recommended internal owners:

```text
std.fs.io
std.fs.copy
std.fs.copy.util
std.fs.delete
std.fs.delete.util
std.fs.temp
```

Keep `std.fs.path` and `std.fs.walk` standard if they remain directly supported APIs; move their private helpers to `.util` children. Move `map-merge`, copy-target validation, directory preparation and recursive copying to the copy family. Move recursive deletion to the delete family. The root can later become a facade.

### `std.lib.time`

The current file mixes model types, validation, civil-calendar arithmetic, instant conversion, adjustment, formatting and parsing. Split it into:

```text
std.lib.time.model
std.lib.time.calendar
std.lib.time.calendar.util
std.lib.time.arithmetic
std.lib.time.format
std.lib.time.format.util
std.lib.time.parse
std.lib.time.parse.util
std.lib.time                 facade
```

Private constants and helpers become ordinary public Vars in the relevant internal implementation owner. The facade can then `intern-all` coherent public owners and use `intern-in` for collisions or renames.

### `work.base`

Split the root into at least:

```text
work.base.algebra
work.base.graph
work.base.graph.util
work.base.host-api
work.base.host-api.util
work.base.runtime-api
work.base.definition
work.base                    facade
```

Move graph types and graph construction into `work.base.graph`; move graph ordering/validation helpers into `work.base.graph.util`; move run/reference lifecycle operations into `host-api`; move `run-reference` into its utility owner; move `def.work` into a definition owner. The facade explicitly assembles these stable surfaces.

### `work.agent`

Split host construction, run state, transitions, model-turn/tool-call orchestration and the stable API:

```text
work.agent.model
work.agent.host
work.agent.transition
work.agent.loop
work.agent.api
work.agent                   facade
```

The current private state-transition helpers become ordinary public functions in internal owners and gain direct tests.

### `lang.core`

This is the last major conversion. It currently combines public API assembly, registry/bootstrap effects, pointer registration, runtime selection, script state and grammar-derived macro generation. First split coherent internal owners for API publication, pointer/fragment registration, macro generation and bootstrap. Change generators rather than hand-editing generated language families. Only then can `lang.core` become a strict facade.

### `std.foundation`

Foundation is a bootstrap exception. Its private collection, graph, traversal, macro-expansion and threading helpers should eventually move into embedded internal children, but those children must be present in Rust bootstrap namespace manifests and Java/Rust loader parity before the private definitions are removed. Do this after the ordinary library proves the model.

## Family-level survey

### Mostly mechanical internalisation

The following families should predominantly receive `:role :internal` and public top-level definitions in place:

- `lang.core.*` implementation children;
- `lang.common.*` books, compiler, grammar, emit and preprocess modules;
- `lang.model.v1.*` language specifications, rewrites and PostgreSQL type emitters;
- `lang.runtime.basic.*` and `lang.runtime.postgres.*` implementations;
- `code.vm.*` and `code.translate.clojure`;
- `db.node.*` implementation modules;
- `work.flow.make.*`, `work.flow.task.report`, `work.base.host`, and `work.base.execution.graph`;
- `tool.cli.report`, `tool.cli.verify`, and `tool.metaspec.*` while metaspec converges on `std.typed`;
- `std.typed.*`, `std.block.*`, and `std.substrate.result` implementation children.

These namespaces should not be bulk-published unless their complete surface is deliberately designed as an export owner.

### Standard namespaces needing helper extraction

Likely first-pass `.util` or domain-owner extractions include:

- `std.codec.hex`, `base64`, `url`, and `form`;
- `std.text.diff`;
- `std.logic.datalog`;
- `std.lib.collection` and `std.lib.component`;
- `std.fs.path` and `std.fs.walk` when kept public;
- `tool.sh`, `tool.sh.git`, `tool.sh.docker`, and `tool.sh.tmux`;
- `tool.package`, `tool.vm`, `tool.project`, `tool.runtime`, and `tool.inrepl`;
- `code.deploy` and any root retained as a standard implementation namespace;
- `db.ledger.chain` and public database utility modules.

### Boundaries requiring an external API decision first

The `std.dom.*` and `db.text.*` families contain many implementation-shaped namespaces but do not yet have a single clearly declared porcelain root. Define their supported external surfaces before marking all children internal. Otherwise the migration would accidentally turn existing direct imports into acknowledged-internal dependencies without a replacement API.

## Runtime and tooling changes

### Canonical namespace declaration model

Create one portable namespace-declaration parser, preferably under `std.block.namespace`, that returns:

```hara
{:namespace/name ...
 :namespace/role :standard
 :namespace/requires
 [{:namespace ...
   :alias ...
   :refers ...
   :access false}]}
```

Use it from `tool.lint`, `tool.project`, documentation, packaging and migration tooling. Do not leave separate partial parsers in each tool.

### Java and Rust parity

Both native namespace parsers must:

- accept `:config {:role :standard|:internal|:facade}`;
- default omitted role to `:standard`;
- accept only `:access true` as the new require option;
- retain role and access edges in namespace descriptors;
- validate facade source forms before or during evaluation;
- expose role through namespace introspection;
- produce equivalent errors and metadata.

### Resource ownership

Project resource registration currently records namespace name, path and source. Extend it with `:project/id`, namespace role, source/test status and parsed require/access edges. Access checks compare the requesting and target project owners.

### Lint rules

Add findings for:

```hara
:tool.lint/private-top-level-definition
:tool.lint/private-top-level-macro
:tool.lint/private-top-level-var
:tool.lint/internal-access-unacknowledged
:tool.lint/facade-definition
:tool.lint/facade-non-publication-form
:tool.lint/publication-collision
:tool.lint/intern-all-noncoherent-surface
```

Initially report warnings. After the repository migration, porcelain projects make private top-level definitions, unacknowledged cross-project internal access and non-publication facade forms errors.

### Publication semantics

Strengthen `intern-all` and `intern-in` with deterministic ordering, collision preflight, source provenance and reload reconciliation. A facade is an API manifest in executable source: future growth of an `intern-all` owner must be intentional and reviewable.

### Packaging, docs and editor tooling

- package API manifests include `:standard` and `:facade` namespaces by default;
- internal namespaces remain package-loadable but are not advertised as supported API;
- intentional internal dependencies and re-exports are recorded;
- docs hide internal namespaces by default but can render an implementation view;
- completion/navigation shows internal APIs when the current project owns them or the require declares `:access true`;
- tests retain direct access to internal namespaces in the same project.

## Delivery sequence

1. Add the specification and canonical namespace-declaration data model.
2. Implement Java/Rust parsing for `:role` and `:access true`, with introspection and parity tests.
3. Add lint findings in warning mode and a syntax-aware inventory command.
4. Pilot `std.format` and `workspace`, including `.util`/domain extraction so `intern-all` owners are coherent.
5. Convert mechanical internal leaf families and add direct tests for former private functions.
6. Extract helpers from standard public namespaces.
7. Convert clear assembly roots (`std.typed`, `std.config`, `std.block`, `code.vm`, `code.test`, `tool.lint`, `db.postgres`) to strict facades.
8. Decompose `std.fs`, `std.lib.time`, `work.base`, `work.agent`, `lang.core`, and finally Foundation bootstrap.
9. Generate package API/internal-access manifests and enable hard cross-project enforcement.
10. Make top-level private definitions errors for projects opting into the porcelain namespace model, then enable the policy for Hara itself.

## Governing invariant

> Every implementation Var is public and directly testable in an internal owner. Every supported external Var is selected explicitly by a standard namespace or a porcelain facade. Namespace roles describe stability; `:access true` records intentional coupling; `intern-all` and `intern-in` remain the only publication declarations.
