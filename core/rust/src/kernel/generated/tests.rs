use super::GeneratedNamespaceConfig;
use crate::kernel::parse_forms;

#[test]
fn configures_defaults_exclusions_aliases_and_requires_without_sources() {
    let forms = parse_forms(
        "(:config {:rename {:exclude [bytes] :alias {string text}}}) \
             (:require [hara.lib.string :as s :refer [trim]])",
    )
    .unwrap();
    let config = GeneratedNamespaceConfig::configure(&forms).unwrap();
    let rewritten = config.rewrite(
        parse_forms("(trim (s/trim (text/upper \" x \")))")
            .unwrap()
            .remove(0),
    );
    let display = format!("{rewritten:?}");
    assert!(display.contains("std.foundation.string/trim"));
    assert!(display.contains("std.foundation.string/upper"));
    assert!(display.contains("bytes/count") == false);
    assert!(GeneratedNamespaceConfig::configure(
        &parse_forms("(:require [missing.lib :as x])").unwrap()
    )
    .unwrap_err()
    .contains("missing generated namespace"));
}

#[test]
fn rejects_removed_namespace_clause_without_intrinsics_compatibility() {
    let forms = parse_forms("(:intrinsics :all)").unwrap();
    assert!(GeneratedNamespaceConfig::configure(&forms)
        .unwrap_err()
        .contains("Unsupported ns clause: :intrinsics"));
}

#[test]
fn rejects_removed_builtins_config_option() {
    let forms = parse_forms("(:config {:builtins [+ - = count get]})").unwrap();
    assert!(GeneratedNamespaceConfig::configure(&forms)
        .unwrap_err()
        .contains("Unsupported :config option: :builtins"));
}

#[test]
fn config_role_defaults_validates_and_is_retained() {
    assert_eq!(GeneratedNamespaceConfig::defaults().role(), "standard");
    for role in ["default", "internal", "facade"] {
        let config = GeneratedNamespaceConfig::configure(
            &parse_forms(&format!("(:config {{:role :{role}}})")).unwrap(),
        )
        .unwrap();
        assert_eq!(config.role(), if role == "default" { "standard" } else { role });
    }
    assert!(GeneratedNamespaceConfig::configure(
        &parse_forms("(:config {:role \"internal\"})").unwrap()
    )
    .unwrap_err()
    .contains(":config :role expects :default, :internal, or :facade"));
    assert!(GeneratedNamespaceConfig::configure(
        &parse_forms("(:config {:role :unsupported})").unwrap()
    )
    .unwrap_err()
    .contains(":config :role expects :default, :internal, or :facade"));
}

#[test]
fn require_access_accepts_only_literal_true() {
    let config = GeneratedNamespaceConfig::configure(
        &parse_forms("(:require [std.foundation.string :access true])").unwrap(),
    )
    .unwrap();
    assert!(config.internal_access().contains("std.foundation.string"));
    for value in ["false", "1", ":true", "nil"] {
        let error = GeneratedNamespaceConfig::configure(
            &parse_forms(&format!(
                "(:require [std.foundation.string :access {value}])"
            ))
            .unwrap(),
        )
        .unwrap_err();
        assert!(error.contains(":require :access expects true"), "{error}");
    }
}

#[test]
fn rejects_host_flavors_on_the_rust_runtime() {
    for flavor in ["jvm", "dotnet"] {
        let error = GeneratedNamespaceConfig::configure(
            &parse_forms(&format!(
                "(:flavor :{flavor} [java.lang String]) (:import vendor.numeric.Vector)"
            ))
            .unwrap(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            format!(
                "native/unsupported-flavor: :{flavor} (host flavors are only available on JVM/.NET runtimes)"
            )
        );
    }
}

#[test]
fn wasm_is_not_a_host_flavor() {
    let error =
        GeneratedNamespaceConfig::configure(&parse_forms("(:flavor :wasm)").unwrap()).unwrap_err();
    assert_eq!(
        error,
        "native/unsupported-flavor: :wasm (Wasm modules use :import)"
    );
}

#[test]
fn config_override_omits_selected_foundation_vars() {
    let config = GeneratedNamespaceConfig::configure(
        &parse_forms("(:config {:override [compile pointer]})").unwrap(),
    )
    .unwrap();
    assert!(config.excluded_foundation().contains("compile"));
    assert!(config.excluded_foundation().contains("pointer"));
    assert!(GeneratedNamespaceConfig::configure(
        &parse_forms("(:config {:blank true :override [compile]})").unwrap()
    )
    .unwrap_err()
    .contains("cannot be combined"));
}

#[test]
fn config_only_selects_an_exact_foundation_surface() {
    let config = GeneratedNamespaceConfig::configure(
        &parse_forms("(:config {:only [map reduce]})").unwrap(),
    )
    .unwrap();
    let exposed = config.exposed_foundation().unwrap();
    assert!(exposed.contains("map"));
    assert!(exposed.contains("reduce"));
    assert!(GeneratedNamespaceConfig::configure(
        &parse_forms("(:config {:override [map] :only [reduce]})").unwrap()
    )
    .unwrap_err()
    .contains("cannot be combined"));
    assert!(GeneratedNamespaceConfig::configure(
        &parse_forms("(:config {:blank true :only []})").unwrap()
    )
    .unwrap_err()
    .contains("cannot be combined"));
}

#[test]
fn refer_clojure_is_not_a_hara_namespace_clause() {
    assert!(GeneratedNamespaceConfig::configure(
        &parse_forms("(:refer-clojure :exclude [compile])").unwrap()
    )
    .unwrap_err()
    .contains("Unsupported ns clause: :refer-clojure"));
}

#[test]
fn records_used_namespaces_for_runtime_referral() {
    let config = GeneratedNamespaceConfig::configure_with(
        &parse_forms("(:use code.test)").unwrap(),
        |target| target == "code.test",
    )
    .unwrap();
    assert_eq!(config.required_namespaces(), &["code.test"]);
    assert_eq!(config.used_namespaces(), &["code.test"]);
    assert!(
        GeneratedNamespaceConfig::configure(&parse_forms("(:use [code.test])").unwrap())
            .unwrap_err()
            .contains(":use expects unqualified namespace symbols")
    );
}

#[test]
fn records_lazy_alias_without_an_eager_dependency() {
    let config = GeneratedNamespaceConfig::configure_with(
        &parse_forms("(:require [code.test :as test :lazy true])").unwrap(),
        |target| target == "code.test",
    )
    .unwrap();
    assert!(config.required_namespaces().is_empty());
    assert_eq!(config.lazy_target("test"), Some("code.test"));
    assert_eq!(
        config
            .rewrite(parse_forms("test/run").unwrap().remove(0))
            .to_string(),
        "test/run"
    );
}

#[test]
fn coroutine_aliases_rewrite_to_fiber_control_forms() {
    let mut config = GeneratedNamespaceConfig::defaults();
    config.set_global_aliases([("co".to_owned(), "std.foundation.coroutine".to_owned())]);
    assert_eq!(
        config
            .rewrite(parse_forms("co/yield").unwrap().remove(0))
            .to_string(),
        "std.foundation.coroutine/yield"
    );
    assert_eq!(
        config
            .rewrite(parse_forms("co/await").unwrap().remove(0))
            .to_string(),
        "std.foundation.coroutine/await"
    );
}

#[test]
fn foundation_aliases_are_source_declared_not_runtime_defaults() {
    let config = GeneratedNamespaceConfig::defaults();
    assert!(config
        .aliases()
        .into_iter()
        .all(|(_, namespace)| !namespace.starts_with("std.foundation.")));
    assert_eq!(config.global_alias(), None);

    let declared =
        GeneratedNamespaceConfig::configure(&parse_forms("(:config {:set-global-alias str})").unwrap())
            .unwrap();
    assert_eq!(declared.global_alias(), Some("str"));

    let rebound = GeneratedNamespaceConfig::configure_with(
        &parse_forms("(:require [demo.kernel :as kernel])").unwrap(),
        |target| target == "demo.kernel",
    )
    .unwrap();
    assert!(rebound
        .aliases()
        .contains(&("kernel".into(), "demo.kernel".into())));
}

#[test]
fn set_global_records_qualified_vars_and_rejects_unqualified_vars() {
    let config = GeneratedNamespaceConfig::configure(
        &parse_forms("(:config {:set-global [demo.global/value IColl/start-string]})").unwrap(),
    )
    .unwrap();
    assert_eq!(
        config.declared_global_imports(),
        &["demo.global/value".to_owned(), "IColl/start-string".to_owned()]
    );
    assert!(GeneratedNamespaceConfig::configure(
        &parse_forms("(:config {:set-global [value]})").unwrap()
    )
    .unwrap_err()
    .contains(":config :set-global expects qualified Vars"));
}

#[test]
fn foundation_require_exclusions_remove_implicit_refers() {
    let config = GeneratedNamespaceConfig::configure(
        &parse_forms("(:require [std.foundation :refer :all :exclude [eval-in-ns]])").unwrap(),
    )
    .unwrap();
    assert!(config.excluded_foundation().contains("eval-in-ns"));
}

#[test]
fn native_aliases_are_universal_and_cannot_be_rebound() {
    let config =
        GeneratedNamespaceConfig::configure(&parse_forms("(:config {:blank true})").unwrap())
            .unwrap();
    assert_eq!(
        config
            .rewrite(parse_forms("Iter/iter-map").unwrap().remove(0))
            .to_string(),
        "std.native.Iter/iter-map"
    );
    assert_eq!(
        config
            .rewrite(parse_forms("String/encode-utf8").unwrap().remove(0))
            .to_string(),
        "std.native.String/encode-utf8"
    );
    assert_eq!(
        config
            .rewrite(parse_forms("str/encode-utf8").unwrap().remove(0))
            .to_string(),
        "str/encode-utf8"
    );
    assert_eq!(
        config
            .rewrite(parse_forms("Bytes/slice").unwrap().remove(0))
            .to_string(),
        "std.native.Bytes/slice"
    );
    assert_eq!(
        config
            .rewrite(parse_forms("bytes/slice").unwrap().remove(0))
            .to_string(),
        "bytes/slice"
    );
    assert!(GeneratedNamespaceConfig::configure(
        &parse_forms("(:require [std.native.Maths :as Iter])").unwrap()
    )
    .unwrap_err()
    .contains("Namespace alias already refers to std.native.Iter"));
}

#[test]
fn rewrites_explicit_macro_refers_only_for_macro_expansion() {
    let config = GeneratedNamespaceConfig::configure_with(
        &parse_forms("(:require [app.macros :as macros :refer [ordinary] :refer-macros [expand]])")
            .unwrap(),
        |target| target == "app.macros",
    )
    .unwrap();
    let source = parse_forms("(expand (ordinary (macros/expand 42)))")
        .unwrap()
        .remove(0);
    assert_eq!(
        config.rewrite(source.clone()).to_string(),
        "(expand (app.macros/ordinary (app.macros/expand 42)))"
    );
    assert_eq!(
        config.rewrite_for_macroexpand(source).to_string(),
        "(app.macros/expand (app.macros/ordinary (app.macros/expand 42)))"
    );
}
