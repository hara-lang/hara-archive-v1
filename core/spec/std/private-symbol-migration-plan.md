# Private symbol migration plan

Status: **approved symbol placement plan**  
Baseline: `hara-lang/hara@69cd5b7c444b6bfd9c73965b651ae54bd091ac30`

This document is the human-readable view of
[`private-symbol-migrations.edn`](./private-symbol-migrations.edn). It lists
every cross-namespace move approved by the initial `defn-` deprecation survey.
Symbols not listed here either become ordinary public Vars in a namespace
classified as `:internal`, require an owner review, or belong to a
bootstrap/generated migration ledger.

All moves preserve the unqualified name, definition body, metadata, schema,
macro status, and behaviour. API renames or semantic changes require a separate
review.

## P0 — pilots

### `std.format` → `std.format.render`

Kind: `function`. Reason: `:strict-facade`.

- `report`

### `std.format.table` → `std.format.table.util`

Kind: `function`. Reason: `:intern-all-owner-coherence`.

- `column-label`
- `column-value`
- `column-width`
- `normalise-column`
- `realised-columns`
- `record-role`
- `table-text`

### `std.format.terminal` → `std.format.terminal.util`

Kind: `function`. Reason: `:intern-all-owner-coherence`.

- `render-line`

### `std.format.terminal` → `std.format.terminal.util`

Kind: `var`. Reason: `:intern-all-owner-coherence`.

- `+ansi-codes+`
- `+ansi-reset+`
- `+default-options+`

### `workspace.core` → `workspace.transition`

Kind: `function`. Reason: `:intern-all-owner-coherence`.

- `reject`
- `route-area-event`
- `select-area`

## P1 — standard library and clear facades

### `std.block` → `std.block.source`

Kind: `function`. Reason: `:strict-facade`.

- `grep-source`
- `namespace-name`

### `std.block.layout` → `std.block.layout.classify`

Kind: `function`. Reason: `:domain-owner`.

- `is-binding?`
- `is-def?`

### `std.codec.base64` → `std.codec.base64.util`

Kind: `function`. Reason: `:standard-owner-helper`.

- `input-bytes`
- `value-of`

### `std.codec.form` → `std.codec.form.util`

Kind: `function`. Reason: `:standard-owner-helper`.

- `checked-pairs`
- `decode-pair`
- `valid-pair?`

### `std.codec.hex` → `std.codec.hex.util`

Kind: `function`. Reason: `:standard-owner-helper`.

- `digit`
- `input-bytes`

### `std.codec.url` → `std.codec.url.util`

Kind: `function`. Reason: `:standard-owner-helper`.

- `append-character`
- `hex-value`
- `unreserved?`

### `std.config` → `std.config.session`

Kind: `function`. Reason: `:strict-facade`.

- `clear-session`
- `get-session`
- `swap-session`

### `std.dom.diff` → `std.dom.diff.splice`

Kind: `function`. Reason: `:domain-owner`.

- `common-prefix-count`
- `common-suffix-count`
- `diff-list-splice`

### `std.dom.item` → `std.dom.item.ops`

Kind: `function`. Reason: `:domain-owner`.

- `operation-props`

### `std.dom.mock` → `std.dom.mock.state`

Kind: `function`. Reason: `:domain-owner`.

- `mock-props-set!`

### `std.dom.update` → `std.dom.update.splice`

Kind: `function`. Reason: `:domain-owner`.

- `insert-values`
- `remove-values`

### `std.lib.collection` → `std.lib.collection.keys`

Kind: `function`. Reason: `:domain-owner`.

- `transform-map-keys`

### `std.lib.collection` → `std.lib.collection.navigate`

Kind: `function`. Reason: `:macro-support-owner`.

- `nav-check-path`
- `nav-check-predicate-step`
- `nav-check-walk`
- `nav-key-step?`
- `nav-select-body`
- `nav-transform-body`
- `nav-transform-group`

### `std.lib.component` → `std.lib.component.lifecycle`

Kind: `function`. Reason: `:domain-owner`.

- `merged-options`
- `run-hooks`

### `std.fs` → `std.fs.copy`

Kind: `function`. Reason: `:domain-owner`.

- `copy-invalid-target?`
- `copy-tree`
- `map-merge`
- `prepare-directory-target`

### `std.fs` → `std.fs.delete`

Kind: `function`. Reason: `:domain-owner`.

- `delete-tree`

### `std.lib.time` → `std.lib.time.calendar`

Kind: `function`. Reason: `:domain-owner`.

- `civil-from-days`
- `days-from-civil`
- `floor-div`
- `shift-months`
- `value-index`

### `std.lib.time` → `std.lib.time.format`

Kind: `function`. Reason: `:domain-owner`.

- `offset-text`
- `pad`
- `token-at`
- `token-value`

### `std.lib.time` → `std.lib.time.format`

Kind: `var`. Reason: `:domain-owner`.

- `pattern-tokens`

### `std.lib.time` → `std.lib.time.model`

Kind: `function`. Reason: `:domain-owner`.

- `fail`
- `make-civil`
- `valid-offset?`

### `std.lib.time` → `std.lib.time.model`

Kind: `var`. Reason: `:domain-owner`.

- `duration-units`
- `milliseconds-per-day`
- `milliseconds-per-minute`

### `std.lib.time` → `std.lib.time.parse`

Kind: `function`. Reason: `:domain-owner`.

- `digit?`
- `parse-digits`
- `parse-offset`

### `std.logic.datalog` → `std.logic.datalog.solve`

Kind: `function`. Reason: `:domain-owner`.

- `instantiate`
- `rule-facts`
- `solve-bindings`
- `solve-negative`
- `solve-positive`

### `std.logic.datalog` → `std.logic.datalog.unify`

Kind: `function`. Reason: `:domain-owner`.

- `match-tuple`
- `matching-bindings`
- `unify-term`
- `walk`

### `std.logic.datalog` → `std.logic.datalog.util`

Kind: `function`. Reason: `:domain-owner`.

- `member?`

### `std.text.diff` → `std.text.diff.algorithm`

Kind: `function`. Reason: `:domain-owner`.

- `deltas-from-edits`
- `edit-script`
- `flush-delta`
- `lcs-table`

### `std.text.diff` → `std.text.diff.patch`

Kind: `function`. Reason: `:domain-owner`.

- `apply-deltas`

### `tool.lint` → `tool.lint.api`

Kind: `function`. Reason: `:strict-facade`.

- `completed`
- `lint-file`
- `lint-project`
- `lint-scans`
- `lint-source`

## P2 — work, database, tools, and runtime

### `code.test` → `code.test.api`

Kind: `macro`. Reason: `:strict-facade`.

- `capture`
- `contains-in`
- `fact`
- `fact:all`
- `fact:exec`
- `fact:get`
- `fact:global`
- `fact:ns`
- `fact:remove`
- `fact:setup`
- `fact:setup?`
- `fact:symbol`
- `fact:teardown`
- `fact:template`
- `just-in`

### `db.postgres` → `db.postgres.lifecycle`

Kind: `function`. Reason: `:strict-facade`.

- `admin-options`
- `connection-options`
- `run-lifecycle`
- `start-pg`
- `start-pg-raw`
- `stop-pg`
- `stop-pg-raw`

### `lang.runtime.basic` → `lang.runtime.basic.registry`

Kind: `var`. Reason: `:load-effect-owner`.

- `+verify-registry+`

### `work.agent` → `work.agent.host`

Kind: `function`. Reason: `:domain-owner`.

- `next-event-id`
- `next-run-id`
- `reference-id`
- `run-record`
- `update-run!`

### `work.agent` → `work.agent.loop`

Kind: `function`. Reason: `:domain-owner`.

- `process-response`

### `work.agent` → `work.agent.transition`

Kind: `function`. Reason: `:domain-owner`.

- `complete!`
- `fail!`
- `public-state`
- `ready!`
- `transition!`

### `work.base` → `work.base.graph`

Kind: `function`. Reason: `:domain-owner`.

- `assert-graph-definitions!`
- `assert-graph-dependencies!`
- `contains-value?`
- `graph-input-keys`
- `graph-node-keys`
- `graph-node-work`
- `graph-order`
- `graph-order-loop`
- `graph-ready?`

### `work.base` → `work.base.host-api`

Kind: `function`. Reason: `:domain-owner`.

- `run-reference`

## Per-move validation

Each implementation PR must:

1. move the complete definition and metadata without rewriting it;
2. update every qualified reference;
3. add direct tests against the new internal owner;
4. retain supported calls through the existing standard or facade namespace;
5. compare sorted public symbols before and after;
6. compare `:doc`, `:arglists`, `:schema`, `:macro`, `:dynamic`,
   `:deprecated`, and `:added` metadata where present;
7. run focused library tests and JVM/Rust loader checks when the namespace is
   embedded or bootstrap-visible;
8. mark the corresponding EDN entry complete.

The source symbol must not remain as an untracked forwarding definition. A
compatibility publication, when required, is expressed with `intern-in`.
