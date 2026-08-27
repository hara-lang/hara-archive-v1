# Workflow definitions

`work.flow` is the registry and compilation layer between named declarations
and ordinary work values. It does not schedule or execute work.

```text
work.base              portable work values and def.work
work.flow              registered descriptors and atomic redefinition
work.flow.task         callable task flow
work.flow.make         reloadable make-plan flow
```

## Declaration

There is one declaration form:

```hara
(ns app.tasks
  (:require [work.base :refer [def.work]]
            [work.flow.task]))

(def.work lint
  [:task]
  {:main {:fn lint-project
          :argcount 1}})
```

The flow path is explicit and is the only dispatch key. A definition containing
the obsolete `:workflow` key is rejected. Flow namespaces are required
explicitly and register their descriptor when loaded; `work.base` does not
auto-load built-in or third-party flows.

Descriptor defaults merge with each definition locally. There is no
namespace-local profile registry. Reusable variants register another explicit
path rather than installing ambient namespace policy.

## Task flow

`work.flow.task` registers `[:task]`. Its definition compiles through the task
template into an ordinary callable `IWork` value and preserves the historical
zero-to-four argument convention.

Task redefinition compiles a complete candidate and replaces the previous work
value only after compilation succeeds.

### `code.test` task compatibility

`code.test/run` is a test-domain adapter over the `[:task]` engine. It keeps the
Foundation zero-to-three selector surface and omits the generic lookup
position:

```hara
(code.test/run)                         ; current namespace
(code.test/run '[std])                  ; namespace selector
(code.test/run '[std] params)           ; selector and params
(code.test/run '[std] params environment)
```

The selector normalizer is shared as
`work.flow.task.selector/invocation-options`. Scalars match by prefix, regular
expressions search, sets use exact membership, lists are conjunctions, and
vectors are disjunctions. An explicit runtime remains a leading argument, and
the runtime/work options remain the final argument.

When no observer is supplied, `code.test` installs the shared task-report
observer. ANSI output is enabled by default, `:no-color` disables it, `:ansi`
overrides the default, and `:no-report` suppresses the observer. The result is
the shared task report document rather than a separate test execution graph.

### `code.manage` task compatibility

`code.manage` uses the same task template and selector surface as `code.test`.
Each operation remains an ordinary task value, while its namespace-owned unit
items are listed and filtered by `work.flow.task.engine`:

```hara
(code.manage/analyse)                         ; current namespace
(code.manage/analyse 'code.manage)            ; namespace selector
(code.manage/analyse ['code 'std] params)     ; selector and params
(code.manage/analyse 'code.manage params env) ; selector, params, environment
```

The direct zero-argument form is scoped to the current namespace. An explicit
selector receives the complete operation catalog and is then applied by the
shared task engine; scalar selectors use prefix matching and vectors are
disjunctions. The selector position is not a lookup position. CLI execution
uses `:all` when no namespace is supplied, a scalar for one namespace, and a
vector for multiple namespaces. The task item identifier is the declared Hara
namespace, so direct, CLI, and composed calls select the same unit.

Operation parameters stay in the task input options and are passed to the
atomic `code.manage.unit.*` tasklet. `plan` and `run` therefore share the same
task report boundary and selection semantics.

## Make flow

`work.flow.make` registers `[:make]`:

```hara
(ns build
  (:require [work.base :refer [def.work]]
            [work.flow.make :as make]))

(def.work +project+
  [:make]
  {:root "."
   :trigger-policy :manual
   :compile-entry compile-entry
   :default [{:id :assets}]
   :sections {:docs [{:id :guide}]}
   :triggers ['app.core]})
```

A make definition compiles to an immutable plan containing target work graphs,
source specifications, triggers, and its normalized definition. Its public
value is a live host. Successful redefinition preserves host identity and
running state while atomically installing the new plan, revision, status, and
trigger receipts.

Make execution remains explicit through `work.flow.make/run`, `build`, and
`clean`. Trigger installation defaults to `:on-define`; a definition can select
`:trigger-policy :manual` and use `start!` and `stop!`.

## Flow descriptor contract

A flow is an ordinary registered map:

```hara
{:flow/path       [:family]
 :flow/version    1
 :flow/product    :work-or-host
 :flow/defaults   {...}
 :flow/extends    [:parent]
 :flow/merge      merge-function
 :flow/configure  configure-function
 :flow/normalise  normalise-function
 :flow/compile    compile-function
 :flow/reconcile  reconcile-function
 :flow/invoke     invoke-function}
```

`def.work` qualifies the declared name and delegates to the descriptor selected
by its path. `work.flow/define!` updates the definition registry only after
normalization, compilation, and reconciliation succeed. This is the shared
atomic boundary for replaceable task values and identity-preserving make hosts.
