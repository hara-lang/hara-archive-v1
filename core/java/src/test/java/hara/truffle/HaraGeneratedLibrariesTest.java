package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Value;
import org.graalvm.polyglot.io.IOAccess;
import org.junit.Test;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;

public class HaraGeneratedLibrariesTest {
  @Test
  public void removedWorkCompatibilityNamespacesCannotBeRequired() {
    try (Context context = context()) {
      assertThrows(
          PolyglotException.class,
          () -> context.eval(HaraLanguage.ID, "(require 'std.work)"));
      assertThrows(
          PolyglotException.class,
          () -> context.eval(HaraLanguage.ID, "(require 'std.work.recipe)"));
      assertThrows(
          PolyglotException.class,
          () -> context.eval(HaraLanguage.ID, "(require 'code.test.selector)"));
    }
  }

  @Test
  public void foundationBootstrapFamilyIsExactlySixNamespaces() {
    Map<String, String> libraries = HaraBuiltinCatalog.GENERATED_LIBRARIES;
    assertEquals(
        "production Foundation family is the root plus exactly five libraries",
        Set.of(
            "std.foundation.string",
            "std.foundation.coroutine",
            "std.foundation.promise",
            "std.foundation.bytes",
            "std.foundation.pretty"),
        new HashSet<>(libraries.values()));
  }

  @Test
  public void nestedLookupDoesNotConsumeItsPath() {
    try (Context context = context()) {
      assertEquals(
          ",",
          context
              .eval(HaraLanguage.ID, "(get-in {:default {:common {:sep \",\"}}} [:default :common :sep])")
              .asString());
    }
  }

  @Test
  public void emitterTypePredicatesAreAvailable() {
    try (Context context = context()) {
      assertTrue(context.eval(HaraLanguage.ID, "(char? \\a)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(list? '(a b))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(not (list? '[a b]))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(pair? (first {:a 1}))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(not (uuid? :a))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(not (regexp? :a))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(fn? (fn [value] value))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(fn? :a)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(fn? {:a 1})").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(fn? #{:a})").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(function? (fn [value] value))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(function? inc)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(not (function? :a))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(not (function? {:a 1}))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(not (function? #{:a}))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(nil? (resolve 'callable?))").asBoolean());
    }
  }

  @Test
  public void uuidIsExposedThroughBaseWithPortableTypeIdentity() {
    try (Context context = context()) {
      assertEquals(
          "[true :std.native.UUID \"00000000-0000-0000-0000-000000000000\" \"00000000-0000-0000-0000-000000000001\"]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [random (Base/uuid) "
                      + "fixed (Base/uuid \"00000000-0000-0000-0000-000000000000\") "
                      + "bits (Base/uuid 0 1)] "
                      + "[(uuid? random) (type random) (str fixed) (str bits)])")
              .toString());
    }
  }

  @Test
  public void uuidInputsAndFoundationPredicateArePortable() {
    try (Context context = context()) {
      assertEquals(
          "[true true true true false true true true true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [byte-uuid (Base/uuid (std.native.Bytes/new 1 2 -1)) "
                      + "keyword (Base/uuid :demo/value) "
                      + "halves (Base/uuid 0 1)] "
                      + "[(std.foundation/uuid? (Base/uuid \"00000000-0000-0000-0000-000000000000\")) "
                      + " (std.foundation/uuid? byte-uuid) "
                      + " (std.foundation/uuid? keyword) "
                      + " (std.foundation/uuid? halves) "
                      + " (std.foundation/uuid? :demo/value) "
                      + " (= (Base/uuid \"00000000-0000-0000-0000-000000000000\") "
                      + "    (Base/uuid \"00000000-0000-0000-0000-000000000000\")) "
                      + " (= byte-uuid (Base/uuid \"4f989b1a-c8e4-3ab1-9569-6571104cfb67\")) "
                      + " (= keyword (Base/uuid \"00000000-6d44-1e45-0000-000006ac9171\")) "
                      + " (= halves (Base/uuid \"00000000-0000-0000-0000-000000000001\"))])")
              .toString());
    }
  }

  @Test
  public void foundationMembershipAndNumericPredicatesStayCanonical() {
    try (Context context = context()) {
      assertTrue(context.eval(HaraLanguage.ID, "(nil? (resolve 'contains?))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(nil? (resolve 'decimal?))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(function? has?)").asBoolean());
    }
  }

  @Test
  public void foundationDerivedFunctionsAndNativeConversionFastPathsAgree() {
    try (Context context = context()) {
      assertEquals(
          "[true true true {:tag :vector}]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(= {:a 2 :b 3} (reduce-kv (fn [out key value] (assoc out key (+ value 1))) {} {:a 1 :b 2})) "
                      + " (= {:b 2} (select-keys {:a 1 :b 2} [:b :missing])) "
                      + " (= {:a 1 :b 3} (merge {:a 1 :b 2} {:b 3})) "
                      + " (meta (vec (with-meta (vector 1) {:tag :vector})))]")
              .toString());
    }
  }

  @Test
  public void protocolPredicatesAndPairsUseCanonicalCapabilities() {
    try (Context context = context()) {
      assertEquals(
          "[true true true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(coll? {}) (counted? []) (pair? (first {:a 1}))]")
              .toString());
      assertTrue(context.eval(HaraLanguage.ID, "(map-entry? (first {:a 1}))").asBoolean());
      assertEquals(
          ":std.native.MapEntry",
          context.eval(HaraLanguage.ID, "(type (first {:a 1}))").toString());
      assertEquals(
          "[false true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(defprotocol Ready (ready [self])) "
                      + "(defstruct Box [value]) "
                      + "(def before (satisfies? Ready (Box 1))) "
                      + "(extend-type Box Ready (ready [self] (:value self))) "
                      + "[before (satisfies? Ready (Box 1))]")
              .toString());
      assertErrorContains(context, "(collection? [])", "Unbound symbol");
    }
  }

  @Test
  public void portableTypeReturnsCanonicalAndNamedKeywords() {
    try (Context context = context()) {
      assertEquals(
          "[:std.native.Nil :std.native.Long :std.native.BigInteger :std.native.Float :std.native.String :std.native.Keyword "
              + ":std.native.Symbol :std.native.Vector :std.native.Vector :std.native.HashMap "
              + ":std.native.OrderedSet :std.native.Pointer :std.native.Function :std.native.Atom :std.native.Vector "
              + ":std.native.Vector :std.native.Vector :std.native.RegExp]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(type nil) (type 1) (type 9223372036854775808) (type 1.5) (type \"x\") (type :x) "
                      + "(type 'x) (type []) (type (vector)) (type {}) "
                      + "(type #{}) (type #ptr {:context :kernel}) (type (fn [x] x)) "
                      + "(type (atom 0)) (std.foundation/type []) "
                      + "(type [1 2 3 4 5 6 7 8]) (type [1 2 3 4 5 6 7 8 9]) "
                      + "(type #\"x\")]")
              .toString());
      assertEquals(
          "[:geometry.Point :geometry.Cursor :std.native.StructType :std.native.MutableType]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns geometry) (defstruct Point [x y]) (defmutable Cursor [x y]) "
                      + "[(type (Point 1 2)) (type (Cursor 1 2)) (type Point) (type Cursor)]")
              .toString());
      assertEquals(
          "[true false true true true true false false]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(vector? []) (pair? [1 2]) (pair? (pair 1 2)) "
                      + "(map-entry? (pair 1 2)) (vector? [1 2 3 4 5 6 7 8]) "
                      + "(vector? [1 2 3 4 5 6 7 8 9]) (pair? (vector 1 2)) "
                      + "(pair? (list 1 2))]")
              .toString());
      assertEquals(
          "[true 2 :missing]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(lookupable? [1 2]) (get [1 2] 1) (get [] 0 :missing)]")
              .toString());
    }
  }

  @Test
  public void typedSchemaValuesSeparateDataOriginsAndVarContracts() {
    try (Context context = context()) {
      assertEquals(
          "[true true true :primitive true true true true true true true true true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns schema.runtime) "
                      + "(def description [:int]) "
                      + "(defn ^{:schema #'description} customer-name [customer] (:name customer)) "
                      + "(def snapshot-description [:int]) "
                      + "(defn ^{:schema #'snapshot-description} snapshot-name [customer] (:name customer)) "
                      + "(def snapshot-description [:string]) "
                      + "(let [from-var (schema #'description) from-value (schema description) "
                      + "direct (schema [:int])] "
                      + "[(schema? direct) (= from-var from-value direct) "
                      + "(schema? direct) (Schema/kind direct) "
                      + "(= #'description (Schema/origin from-var)) "
                      + "(= from-var (schema-of #'customer-name)) "
                      + "(= direct (schema-of #'snapshot-name)) "
                      + "(= direct (schema {:kind :primitive :children [:int]})) "
                      + "(= [:int] (Schema/form direct)) (map? (Schema/ast direct)) "
                      + "(= direct (schema direct)) (= direct (schema :int)) "
                      + "(nil? (schema-of #'description))])")
              .toString());
      assertErrorContains(context, "(schema #'customer-name)", "schema expects schema data");
      assertErrorContains(context, "(schema customer-name)", "schema expects schema data");
      assertErrorContains(context, "(schema-of customer-name)", "Schema/of expects a Var");
    }
  }

  @Test
  public void nativeTestCatalogUsesRuntimeRunnerAndTestContext() {
    try (Context context = context()) {
      assertEquals(
          "[true [:code.test :native] :code.test :test "
              + "[:test/run-started :test/fact-started :test/fact-completed :test/run-completed]]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(= Test std.native.Test) (get (Test/catalog) :runners) "
                      + "(get (Test/catalog) :default) (get (Test/catalog) :context) "
                      + "(Test/events)]")
              .toString());
      assertEquals(
          "[:code.test :fast :test :test :code.test]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [config (Test/config {:focus :fast}) "
                      + "context (Test/context config)] "
                      + "[(get config :runner) (get (get config :options) :focus) "
                      + "(IPointer/ptr-context context) (get context :id) "
                      + "(get (get context :config) :runner)])")
              .toString());
      assertErrorContains(
          context, "(Test/config {:runner :native})", "runner is owned by the runtime");
      assertEquals(
          "[true false 7 8 1 true 1 1]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [equal (Test/result \"equal\" 7 7 (Test/compare 7 7)) "
                      + "different (Test/result \"different\" 7 8 (Test/compare 7 8))] "
                      + "[(Test/passed? equal) (Test/passed? different) "
                      + "(Test/actual different) (Test/expected different) "
                      + "(Test/failure-count different) (Test/failure? (Test/failure different 0)) "
                      + "(count (Test/failures different)) (count (Test/failure-seq different))])")
              .toString());
      assertErrorContains(context, "(Test/passed? {:status :error})", "expects a Result");
      assertEquals(
          "[[:left :right] 2 :right true false]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [leaf (fn [code] "
                      + "{:failure/code code :failure/path [] :failure/in [] "
                      + ":failure/actual nil :failure/expected nil "
                      + ":failure/message \"failure\" :failure/context {} "
                      + ":failure/children []}) "
                      + "left (leaf :left) right (leaf :right) "
                      + "parent {:failure/code :parent :failure/path [] :failure/in [] "
                      + ":failure/actual nil :failure/expected nil "
                      + ":failure/message \"parent\" :failure/context {} "
                      + ":failure/children [left right]} "
                      + "result (Result/create :success false {:failures [parent]})] "
                      + "[(vec (map :failure/code (Test/failure-seq result))) "
                      + "(Test/failure-count result) (:failure/code (Test/failure result 1)) "
                      + "(Test/failure? parent) "
                      + "(Test/failure? (assoc parent :failure/children [{}]))])")
              .toString());
    }
    try (Context context =
        Context.newBuilder(HaraLanguage.ID).option("hara.TestRunner", "native").build()) {
      assertEquals(
          "[:native :native]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(get (Test/catalog) :runner) (get (Test/config) :runner)]")
              .toString());
    }
  }

  @Test
  public void nativeTestRunAccumulatesCasesAndKeepsErrorsLocal() {
    try (Context context = context()) {
      String first = context.eval(HaraLanguage.ID,
          "(Test/run [{:name \"one\" :test (fn [] (+ 1 1)) :expected 2}])").toString();
      assertTrue(first, first.contains(":name \"one\""));
      assertTrue(first, first.contains("#hara/Result[:success true"));
      String cumulative = context.eval(HaraLanguage.ID,
          "(Test/run [{:name \"two\" :test (fn [] (throw \"boom\")) :expected 2}])")
          .toString();
      assertTrue(cumulative, cumulative.contains(":name \"one\""));
      assertTrue(cumulative, cumulative.contains(":name \"two\""));
      assertTrue(cumulative, cumulative.contains("#hara/Result[:error"));
      assertEquals(cumulative, context.eval(HaraLanguage.ID, "(Test/run [])").toString());
      String malformed = context.eval(HaraLanguage.ID, "(Test/run [{} 1])").toString();
      assertTrue(malformed, malformed.contains("case requires :test"));
      assertTrue(malformed, malformed.contains("case must be a map"));
    }
  }

  @Test
  public void nativeTestRunAwaitsPromiseResults() {
    try (Context context = context()) {
      String result = context.eval(HaraLanguage.ID,
          "(Test/run [{:name \"async\" "
              + ":test (fn [] (promise/delay 1 (fn [] 42))) "
              + ":expected 42}])").toString();
      assertTrue(result, result.contains(":name \"async\""));
      assertTrue(result, result.contains("#hara/Result[:success true"));
      assertTrue(result, result.contains(":actual 42"));
    }
  }

  @Test
  public void promiseCatchPreservesStructuredRejectionValues() {
    try (Context context = context()) {
      assertEquals(
          ":response",
          context
              .eval(
                  HaraLanguage.ID,
                  "(deref (std.foundation.promise/catch "
                      + "(std.foundation.promise/new "
                      + "(fn [resolve reject] (reject {:kind :response}))) "
                      + "(fn [error] (:kind error))))")
              .toString());
    }
  }

  @Test
  public void nativeTestRunAcceptsAFunctionAwareChecker() {
    try (Context context = context()) {
      String checked = context.eval(HaraLanguage.ID,
          "(Test/run [{:name \"checked\" :meta {:refer (quote demo/value)} "
              + ":test (fn [] 7) :expected odd?}] "
              + "(fn [thunk expected] (let [actual (thunk)] "
              + "(Test/result \"checker\" actual :predicate (Test/compare (expected actual) true)))))")
          .toString();
      assertTrue(checked, checked.contains(":name \"checked\""));
      assertTrue(checked, checked.contains("#hara/Result[:success true"));
      assertTrue(checked, checked.contains(":meta {:refer demo/value}"));

      String failures = context.eval(HaraLanguage.ID,
          "(Test/run [{:name \"throws\" :test (fn [] 1) :expected 1} "
              + "{:name \"continues\" :test (fn [] 2) :expected 2}] "
              + "(fn [thunk expected] (throw \"checker boom\")))")
          .toString();
      assertTrue(failures, failures.contains(":name \"throws\""));
      assertTrue(failures, failures.contains(":name \"continues\""));
      assertEquals(2, failures.split("#hara/Result\\[:error", -1).length - 1);

      String malformed = context.eval(HaraLanguage.ID,
          "(Test/run [{:name \"malformed\" :test (fn [] 1) :expected 1}] "
              + "(fn [thunk expected] true))")
          .toString();
      assertTrue(malformed, malformed.contains("check function must return a Result"));
    }
  }

  @Test
  public void nativeTestRunSupportsLifecycleMaps() {
    try (Context context = context()) {
      String result = context.eval(HaraLanguage.ID,
          "(let [events (atom [])] "
              + "[(Test/run [{:name \"case\" :test (fn [] (swap! events conj :case) 1) :expected 1}] "
              + "{:setup (fn [] (swap! events conj :setup)) "
              + ":teardown (fn [] (swap! events conj :teardown))}) @events])").toString();
      assertTrue(result, result.contains("#hara/Result[:success true"));
      assertTrue(result, result.contains("[:setup :case :teardown]"));

      String failure = context.eval(HaraLanguage.ID,
          "(let [events (atom [])] "
              + "[(Test/run [{:name \"skipped\" :test (fn [] (swap! events conj :case)) :expected nil}] "
              + "{:setup (fn [] (throw \"setup boom\")) "
              + ":teardown (fn [] (swap! events conj :teardown) (throw \"teardown boom\"))}) @events])")
          .toString();
      assertTrue(failure, failure.contains(":phase :setup"));
      assertTrue(failure, failure.contains(":phase :teardown"));
      assertTrue(failure, failure.contains("[:teardown]"));
      assertTrue(failure, !failure.contains(":name \"skipped\""));
    }
  }

  @Test
  public void renameCanExcludeAndRenameGeneratedAliases() {
    try (Context context = context()) {
      assertEquals(
          "HARA",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns app (:config {:rename {:exclude [bytes] :alias {string text}}})) "
                      + "(text/upper \"hara\")")
              .asString());
      PolyglotException missing =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(bytes/count (bytes 1))"));
      assertTrue(missing.getMessage().contains("Unbound symbol: bytes/count"));
      assertEquals("x", context.eval(HaraLanguage.ID, "(str \"x\")").asString());
    }
  }

  @Test
  public void sourceOwnedGlobalAliasesSurviveNamespaceSelection() {
    try (Context context = context()) {
      assertEquals(
          "[42 demo.global]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns demo.global (:config {:set-global-alias global})) "
                      + "(defn value [] 42) "
                      + "(ns demo.consumer) "
                      + "[(global/value) (get (ns-alias-state 'global) :target)]")
              .toString());
    }
  }

  @Test
  public void localRequiresOverrideGlobalAliases() {
    try (Context context = context()) {
      assertEquals(
          "[7 demo.other]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns demo.global (:config {:set-global-alias global})) "
                      + "(defn value [] 42) "
                      + "(ns demo.other) (defn value [] 7) "
                      + "(ns demo.consumer (:require [demo.other :as global])) "
                      + "[(global/value) (get (ns-alias-state 'global) :target)]")
              .toString());
    }
  }

  @Test
  public void globalAliasValidationMatchesRustContract() {
    try (Context context = context()) {
      assertErrorContains(
          context,
          "(ns invalid.vector (:config {:set-global-alias [value]}))",
          ":config :set-global-alias expects an unqualified symbol");
      assertErrorContains(
          context,
          "(ns invalid.qualified (:config {:set-global-alias other/value}))",
          ":config :set-global-alias expects an unqualified symbol");
      assertErrorContains(
          context,
          "(ns invalid.reserved (:config {:set-global-alias -}))",
          ":config :set-global-alias is reserved: -");
    }
  }

  @Test
  public void requireAccessAcceptsOnlyLiteralTrue() {
    try (Context context = context()) {
      assertEquals(
          ":internal",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns access.internal (:config {:role :internal})) "
                      + "(ns access.consumer (:require [access.internal :access true])) "
                      + "(get (Runtime/namespace 'access.internal) :namespace/role)")
              .toString());
      assertErrorContains(
          context,
          "(ns access.false (:require [access.internal :access false]))",
          ":require :access expects true");
      assertErrorContains(
          context,
          "(ns access.number (:require [access.internal :access 1]))",
          ":require :access expects true");
      assertErrorContains(
          context,
          "(ns access.keyword (:require [access.internal :access :true]))",
          ":require :access expects true");
    }
  }

  @Test
  public void conflictingGlobalAliasRegistrationRollsBack() {
    try (Context context = context()) {
      context.eval(
          HaraLanguage.ID,
          "(ns demo.stable (:config {:set-global-alias shared})) (defn value [] 42)");
      assertErrorContains(
          context,
          "(ns demo.conflict (:config {:set-global-alias shared}))",
          "Global namespace alias already refers to demo.stable: shared");
      assertEquals(
          "[42 demo.stable]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns demo.consumer) "
                      + "[(shared/value) (get (ns-alias-state 'shared) :target)]")
              .toString());
    }
  }

  @Test
  public void setGlobalImportsUseTerminalNamesAndCompactProtocolSymbols() {
    try (Context context = context()) {
      context.eval(
          HaraLanguage.ID,
          "(ns demo.global (:config {:set-global [demo.global/value]})) "
              + "(def value 42) "
              + "(ns demo.protocol (:config {:set-global [IColl/start-string IMetadata/metatype]}))");
      assertEquals("42", context.eval(HaraLanguage.ID, "value").toString());
      assertEquals("[", context.eval(HaraLanguage.ID, "(start-string [])").toString());
      assertEquals("MAP", context.eval(HaraLanguage.ID, "(metatype {:value 1})").toString());
    }
  }

  @Test
  public void generatedLibrariesAlsoSupportRequireAsAndRefer() {
    try (Context context = context()) {
      assertEquals(
          "x",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns app (:config {:rename {:exclude [string]}}) "
                      + "(:require [std.foundation.string :as text :refer [trim]])) "
                      + "(trim (text/trim \" x \"))")
              .asString());
    }
  }

  @Test
  public void onlyPortableFoundationShorthandsAreAutomatic() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns sample.kernel) (def value 42) "
                      + "(ns app (:require [sample.kernel :as kernel])) kernel/value")
              .asLong());
      assertEquals("x", context.eval(HaraLanguage.ID, "(str \"x\")").asString());
      assertErrorContains(context, "(json/read \"null\")", "Unbound symbol: json/read");
    }
  }

  @Test
  public void definitionsShadowReferredVarsWithoutMutatingTheirOwners() {
    try (Context context = context()) {
      assertEquals(
          "[99 3]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns app (:config {:override [+]})) "
                      + "(defn + [a b] 99) [(+ 1 2) (std.foundation/+ 1 2)]")
              .toString());

      assertEquals(
          "[99 3]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns protected (:config {:blank true}) "
                      + "(:require [std.foundation :refer [+]])) "
                      + "(defn + [a b] 99) [(+ 1 2) (std.foundation/+ 1 2)]")
              .toString());
      assertEquals(
          "[42 0]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns protected-declare (:config {:blank true}) "
                      + "(:require [std.foundation :refer [identity]])) "
                      + "(declare identity) (defn identity [value] 42) "
                      + "[(identity 0) (std.foundation/identity 0)]")
              .toString());
    }
  }

  @Test
  public void configOverridesOmitSelectedFoundationVars() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns app.runtime (:config {:override [Runtime]})) "
                      + "(defstruct Runtime [value]) "
                      + "(:value (app.runtime/Runtime 42))")
              .asLong());
      assertErrorContains(
          context,
          "(ns legacy (:refer-clojure :exclude [Runtime]))",
          "Unsupported ns clause: :refer-clojure");
      assertErrorContains(
          context,
          "(ns contradictory (:config {:blank true :override [Runtime]}))",
          "cannot be combined with :override");
    }
  }

  @Test
  public void configOnlySelectsOnlyNamedFoundationVars() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns exposed (:config {:only [identity]})) (identity 42)")
              .asLong());
      assertErrorContains(context, "(count [1 2])", "Unbound symbol: count");
      assertErrorContains(
          context,
          "(ns mixed (:config {:override [map] :only [inc]}))",
          "cannot be combined with :only");
    }
  }

  @Test
  public void namespaceRolesAreParsedRetainedAndRedeclared() {
    try (Context context = context()) {
      assertEquals(
          "[:standard :internal :facade]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns role.standard) "
                      + "(ns role.internal (:config {:role :internal})) "
                      + "(ns role.facade (:config {:role :facade})) "
                      + "[(get (Runtime/namespace 'role.standard) :namespace/role) "
                      + " (get (Runtime/namespace 'role.internal) :namespace/role) "
                      + " (get (Runtime/namespace 'role.facade) :namespace/role)]")
              .toString());
      assertEquals(
          ":standard",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns role.internal) "
                      + "(get (Runtime/namespace 'role.internal) :namespace/role)")
              .toString());
      assertErrorContains(
          context,
          "(ns role.invalid (:config {:role :unsupported}))",
          ":config :role expects :default, :internal, or :facade");
    }
  }

  @Test
  public void requireExclusionsSurviveLoadingLaterSourceNamespaces() {
    try (Context context =
        Context.newBuilder(HaraLanguage.ID).allowIO(IOAccess.ALL).build()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns app.require-order "
                      + "(:require [std.foundation :refer :all :exclude [filter]] "
                      + "          [work.base.model :as work-model])) "
                      + "(defn filter [value] 42) (filter :value)")
              .asLong());
    }
  }

  @Test
  public void lexicalBindingsShadowCallableFoundationVars() {
    try (Context context = context()) {
      assertEquals(
          "[99 77]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(let [+ (fn [a b] 99)] (+ 1 2)) "
                      + " (let [count (fn [value] 77)] (count [1 2 3]))]")
              .toString());
    }
  }

  @Test
  public void namespaceUseLoadsAndRefersVarsAndMacros() {
    try (Context context = context()) {
      assertEquals(
          84,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns demo.use-lib) "
                      + "(def answer 42) "
                      + "(defmacro twice [form] `(+ ~form ~form)) "
                      + "(ns demo.use-app (:use demo.use-lib)) "
                      + "(twice answer)")
              .asLong());
      assertErrorContains(
          context,
          "(ns demo.bad-use (:use [demo.use-lib]))",
          ":use expects unqualified namespace symbols");
    }
  }

  @Test
  public void foundationNamespaceCombinesJavaAndHalSymbols() {
    try (Context context = context()) {
      assertEquals(
          -1,
          context
              .eval(
                  HaraLanguage.ID, "(ns app (:require [std.foundation :as core])) (core/bit-not 0)")
              .asLong());
      assertEquals(
          1,
          context.eval(HaraLanguage.ID, "(std.foundation/count [1])").asLong());
      assertEquals(
          42,
          context.eval(HaraLanguage.ID, "((std.foundation/comp inc inc) 40)").asLong());
    }
  }

  @Test
  public void renameRejectsUnknownConflictingAndDuplicateConfiguration() {
    try (Context context = context()) {
      assertErrorContains(
          context,
          "(ns a (:config {:rename {:exclude [unknown]}}))",
          "Unknown Foundation library");
      assertErrorContains(
          context,
          "(ns b (:config {:rename {:exclude [bytes] :alias {bytes data}}}))",
          "both excluded and aliased");
      assertErrorContains(
          context,
          "(ns c (:config {:rename {:alias {string data bytes data}}}))",
          "Duplicate Foundation library alias target");
      assertErrorContains(
          context, "(ns d (:config {}) (:config {}))", "only one :config clause");
      assertErrorContains(
          context,
          "(ns e (:config {:rename {:unexpected true}}))",
          "Unsupported :config :rename option");
    }
  }

  @Test
  public void completionIncludesGeneratedAliasesAndMarkerMethods() {
    try (Context context = context()) {
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(Iter/iter-any? (fn [x] (= x \"str/trim\")) (current-symbols))")
              .asBoolean());
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(not (Iter/iter-any? (fn [x] (= x \"str/len\")) (current-symbols)))")
              .asBoolean());
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(Iter/iter-any? (fn [x] (= x \"co/resume\")) (current-symbols))")
              .asBoolean());
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(Iter/iter-any? (fn [x] (= x \"push-last\")) (current-symbols))")
              .asBoolean());
    }
  }

  @Test
  public void completionOnlyQualifiesSymbolsOwnedByRequiredAliases() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(ns sample.walk) (def own-symbol 1)");
      context.eval(HaraLanguage.ID, "(ns user (:require [sample.walk :as walk]))");
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(and (Iter/iter-any? (fn [x] (= x \"walk/own-symbol\")) (current-symbols)) "
                      + "     (not (Iter/iter-any? (fn [x] (= x \"walk/+\")) (current-symbols))) "
                      + "     (not (Iter/iter-any? (fn [x] (= x \"walk/ILookup\")) (current-symbols))))")
              .asBoolean());
    }
  }

  @Test
  public void completionRanksPublicVarsBeforeDeterministicallyOrderedHelpers() {
    try (Context context = context()) {
      context.eval(
          HaraLanguage.ID,
          "(ns completion.order) "
              + "(def zebra-helper 1) "
              + "(def ^{:public true} recommended-api 2) "
              + "(def alpha-helper 3) "
              + "(def ^{:public true} advertised-api 4)");
      Value symbols = context.eval(HaraLanguage.ID, "(current-symbols)");
      int advertised = indexOf(symbols, "advertised-api");
      int recommended = indexOf(symbols, "recommended-api");
      int alpha = indexOf(symbols, "alpha-helper");
      int zebra = indexOf(symbols, "zebra-helper");
      assertTrue(advertised >= 0);
      assertTrue(recommended >= 0);
      assertTrue(alpha >= 0);
      assertTrue(zebra >= 0);
      assertTrue(advertised < recommended);
      assertTrue(recommended < alpha);
      assertTrue(alpha < zebra);
    }
  }

  @Test
  public void dotCallsAreRestrictedToMarkedArraysAndObjects() {
    try (Context context = context()) {
      assertEquals(
          6,
          context
              .eval(HaraLanguage.ID, "(Arr/fold-left (array 1 2 3) (fn [out x] (+ out x)) 0)")
              .asLong());
      assertEquals(
          3,
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [a (array 1 2 3 4)] (Arr/get (Arr/filter a (fn [x] (> x 2))) 0))")
              .asLong());
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [o (object \"answer\" 41)] (Obj/set o \"answer\" 42) "
                      + "(Obj/get o \"answer\"))")
              .asLong());
      PolyglotException denied =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(. [1 2] (get 0))"));
      assertTrue(
          denied.getMessage().contains("only supported on values created by array or object"));
    }
  }

  @Test
  public void bitOperationsUseSignedThirtyTwoBitSemantics() {
    try (Context context = context()) {
      assertEquals(2, context.eval(HaraLanguage.ID, "(bit-and 6 3)").asLong());
      assertErrorContains(context, "(bit-and 7 3 1)", "expects two integers");
      assertErrorContains(context, "(bit-or 1 2 4)", "expects two integers");
      assertErrorContains(context, "(bit-xor 1 2 4)", "expects two integers");
      assertEquals(-1, context.eval(HaraLanguage.ID, "(bit-not 0)").asLong());
      assertEquals(-2, context.eval(HaraLanguage.ID, "(bit-shift-right -4 1)").asLong());
      assertEquals(-2147483648L, context.eval(HaraLanguage.ID, "(bit-shift-left 1 31)").asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "(bit-shift-left 1 0)").asLong());
      assertErrorContains(context, "(bit-shift-left 1 -1)", "distance must be in the range 0..31");
      assertErrorContains(context, "(bit-shift-right 1 32)", "distance must be in the range 0..31");
    }
  }

  private static Context context() {
    return Context.newBuilder(HaraLanguage.ID).build();
  }

  private static void assertErrorContains(Context context, String source, String message) {
    PolyglotException error =
        assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, source));
    assertTrue(error.getMessage().contains(message));
  }

  private static int indexOf(Value values, String expected) {
    for (long index = 0; index < values.getArraySize(); index++) {
      if (expected.equals(values.getArrayElement(index).asString())) return (int) index;
    }
    return -1;
  }
}
