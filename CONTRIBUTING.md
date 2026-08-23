# Contributing to Hara

By submitting a contribution, you certify that you have the right to submit it
and license your contribution under the repository's [Apache-2.0](LICENSE) terms.
Do not submit third-party code, assets, or generated material unless its
provenance and license are compatible and recorded in `LICENSES/README.md`.

Use the Developer Certificate of Origin (DCO) sign-off on every commit:

```text
Signed-off-by: Your Name <you@example.com>
```

Git can add it with `git commit -s`. The sign-off certifies the statement in
[developercertificate.org](https://developercertificate.org/).

## Namespace ownership

Do not add top-level `defn-`, `defmacro-`, or private Vars to Hara source.
Implementation functions belong in a namespace declared with `(:config {:role
:internal})`, where they remain directly testable. Supported porcelain
namespaces declare `(:config {:role :facade})`, contain only publication forms,
and use `intern-all` when the complete coherent owner is supported. Use
`intern-in` for selected publication.

Do not make a helper public merely so an `intern-all` facade can reach it. Move
unsupported helpers to an internal owner, and add their corresponding tests
there.

Mark supported, recommended API Vars at their owning definitions with
`^{:public true}`. Autocomplete and documentation tools use this metadata to
prioritize the intended public surface. It does not override `:internal`, grant
access, or publish a Var by itself; facades still use `intern-all` or
`intern-in`, which must preserve the owner's marker. Leave directly testable
implementation helpers unmarked unless they are intentionally recommended API.

## Reversibility and test isolation

Components that own mutable state, caches, registries, processes, or lifecycle
resources must expose an idempotent reset, teardown, or snapshot/restore
boundary. It must return the component to its documented baseline after normal
use, partial initialization, or failure. Tests must establish that baseline and
restore it on every exit path.

Data-format transformations must provide and test their inverse, such as
encode/decode, parse/render, or serialize/deserialize. Test both round-trip
directions when the formats are equivalent. Lossy or canonicalizing transforms
must document the loss, preserve enough source or provenance to reconstruct the
prior representation, and prove canonicalization is idempotent.

## Source and test correspondence

Hara source and tests are maintained as pairs. A source namespace in
`core/lib/src` or `core/lib/src-lang` must have a test namespace at the same
relative path in the corresponding `core/lib/test` or `core/lib/test-lang`
root, named `*_test.hal`. Every function or macro, including ordinary
definitions in `:internal` namespaces, must have a corresponding
`^{:refer namespace/symbol}` test block with a real behavioral assertion.

After finishing the source implementation, scaffold the corresponding tests
before writing their bodies:

```shell
hara --project core --offline manage scaffold namespace.name
hara --project core --offline manage scaffold namespace.name --write
```

The first command is a dry-run preview. The second creates or updates the
paired test file. For bootstrap, `std.native.*`, and `std.protocol.*` seams,
use `std.foundation.bootstrap/scaffold` to generate and check the corresponding
`Test/run` blocks.

Scaffolds deliberately produce pending test bodies. Replace every generated
stub with meaningful assertions, run the focused test file, and check the
changed namespace with `code.manage`'s `incomplete`, `unchecked`, or `pedantic`
report. Missing files, missing references, TODO facts, empty `Test/run` blocks,
and unchecked facts all mean the change is not complete.

The scaffold is an inventory, not an author. Read each function and hand-write
its permanent test from the semantic contract. Prefer exact stable values,
branch and boundary cases, expected errors, observable state transitions,
cleanup/reset behavior, and inverse or round-trip properties. Do not substitute
a broad type, truthiness, non-nil, successful transport status, or "does not
throw" check for a more specific domain result. Those checks are sufficient
only when they are the documented behavior.

## Connector issue contracts

Validate an issue contract without GitHub credentials or network access:

```shell
hara --project core --offline manage contract-check path/to/issue-contract.md
```

The command prints deterministic EDN with `:status`, `:valid?`, `:findings`,
and `:summary`, and exits non-successfully when `:valid?` is false. Use
`--complete` only when the work is complete and may use a `Closes` relationship.
Finding codes include `:section/missing`, `:section/duplicate`,
`:section/malformed`, `:section/empty`, `:link/missing`,
`:link/noncanonical`, `:relationship/advances-missing`, and
`:relationship/closes-only`.

Demonstrate that a new or materially changed test detects failure by running it
against the pre-change behavior or a deliberately incorrect candidate or
expectation, observing the focused test fail, then restoring the intended code
and assertion and observing it pass. One corresponding block per function is
the minimum navigable index; add all assertions needed to describe the
behavior.
