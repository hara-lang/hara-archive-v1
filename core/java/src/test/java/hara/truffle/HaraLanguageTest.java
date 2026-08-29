package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.kernel.base.RT;
import hara.spec.SpecRegistry;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashSet;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.HostAccess;
import org.graalvm.polyglot.io.IOAccess;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Value;
import org.junit.Test;

public class HaraLanguageTest {
  @Test
  public void evaluatesLiteralsAndAddition() {
    try (Context context = context()) {
      assertEquals(42, context.eval(HaraLanguage.ID, "(+ 19 23)").asLong());
      assertEquals("hara", context.eval(HaraLanguage.ID, "\"hara\"").asString());
      assertTrue(context.eval(HaraLanguage.ID, "true").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "nil").isNull());
      assertEquals(":ready", context.eval(HaraLanguage.ID, ":ready").toString());
    }
  }

  @Test
  public void displaysCharactersThroughTheDisplayProtocol() {
    try (Context context = context()) {
      assertEquals("\\a", context.eval(HaraLanguage.ID, "(IDisplay/display \\a)").asString());
      assertEquals(
          "\\space", context.eval(HaraLanguage.ID, "(IDisplay/display \\space)").asString());
      assertEquals(
          "\\newline",
          context.eval(HaraLanguage.ID, "(IDisplay/display \\newline)").asString());
      assertEquals("a", context.eval(HaraLanguage.ID, "(str \\a)").asString());
      assertEquals("\\a", context.eval(HaraLanguage.ID, "(pr-str \\a)").asString());
      assertEquals("true", context.eval(HaraLanguage.ID, "(IDisplay/display true)").asString());
      assertEquals("false", context.eval(HaraLanguage.ID, "(IDisplay/display false)").asString());
      assertEquals("true", context.eval(HaraLanguage.ID, "(str true)").asString());
      assertEquals("false", context.eval(HaraLanguage.ID, "(pr-str false)").asString());
      assertTrue(context.eval(HaraLanguage.ID, "(char? (first \"abc\"))").asBoolean());
      assertEquals("a", context.eval(HaraLanguage.ID, "(str (first \"abc\"))").asString());
      assertEquals(
          "\\b", context.eval(HaraLanguage.ID, "(pr-str (get \"abc\" 1))").asString());
    }
  }

  @Test
  public void collectionCategoryPredicatesUsePortableProtocols() {
    try (Context context = context()) {
      assertEquals(
          "[true false true false true false true false true false true false]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(satisfies? IMapType {:a 1}) "
                      + " (satisfies? IMapType [1]) "
                      + " (satisfies? ISetType #{1}) "
                      + " (satisfies? ISetType [1]) "
                      + " (satisfies? ILinearType [1]) "
                      + " (satisfies? ILinearType #{1}) "
                      + " (map? {:a 1}) (map? [1]) "
                      + " (set? #{1}) (set? [1]) "
                      + " (sequential? [1]) (sequential? #{1})]")
              .toString());
    }
  }

  @Test
  public void collectionCategoryPredicatesClassifyAllPortableFamilies() {
    try (Context context = context()) {
      assertEquals(
          "[true true false true true false true true true true true true true false false false true false true false true true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(map? {:a 1}) "
                      + " (map? (std.native.Algo/ordered-map :a 1)) "
                      + " (map? [1]) "
                      + " (set? #{1}) "
                      + " (set? (std.native.Algo/ordered-set 1)) "
                      + " (set? [1]) "
                      + " (sequential? '(1 2)) "
                      + " (sequential? [1 2]) "
                      + " (sequential? [1 2]) "
                      + " (sequential? (std.native.Algo/queue 1 2)) "
                      + " (sequential? (std.native.Algo/deque 1 2)) "
                      + " (sequential? (cons 1 [2])) "
                      + " (sequential? (seq [1 2])) "
                      + " (sequential? (std.native.Algo/ordered-set 1)) "
                      + " (coll? (seq [1 2])) "
                      + " (coll? (iter [1 2])) "
                      + " (seq? (seq [1 2])) "
                      + " (seq? [1 2]) "
                      + " (iter? (iter [1 2])) "
                      + " (iter? [1 2]) "
                      + " (map? (IToMutable/to-mutable {:a 1})) "
                      + " (set? (IToMutable/to-mutable #{1}))]")
              .toString());
    }
  }

  @Test
  public void sequentialAndLinearCategoriesRemainDistinct() {
    try (Context context = context()) {
      assertEquals(
          "[true false true false true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(satisfies? ISequential (seq [1 2])) "
                      + " (satisfies? ILinearType (seq [1 2])) "
                      + " (satisfies? ISequential (cons 1 [2])) "
                      + " (satisfies? ILinearType (cons 1 [2])) "
                      + " (satisfies? ILinearType [1 2])]")
              .toString());
    }
  }

  @Test
  public void notAndCompareAreFoundationOwned() {
    try (Context context = context()) {
      assertEquals(
          "[true false -1 0 1]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(std.foundation/not nil) "
                      + " (std.foundation/not true) "
                      + " (std.foundation/compare 1 2) "
                      + " (std.foundation/compare 2 2) "
                      + " (std.foundation/compare 2 1)]")
              .toString());
    }
  }

  @Test
  public void foundationProtocolPredicatesUseSatisfies() {
    try (Context context = context()) {
      assertEquals(
          "[true true true true true true true true true true true true true true true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(iterable? [1]) "
                      + " (iterator? (iter [1])) "
                      + " (counted? [1]) "
                      + " (reducible? [1]) "
                      + " (indexed? [1]) "
                      + " (associative? {:a 1}) "
                      + " (findable? {:a 1}) "
                      + " (lookupable? {:a 1}) "
                      + " (derefable? (atom 1)) "
                      + " (resettable? (atom 1)) "
                      + " (casable? (atom 1)) "
                      + " (watchable? (atom 1)) "
                      + " (applicable? (pointer {:context :test})) "
                      + " (mutable? (to-mutable (vec [1]))) "
                      + " (persistent? [1])]")
              .toString());
    }
  }

  @Test
  public void pointersAreCanonicalDescriptorsWithContextDispatch() {
    try (Context context = context()) {
      assertEquals(
          "[true :test \"ROOT\" 1 [:id \"ROOT\"]]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(pointer? (pointer {:context :test :id \"ROOT\"})) "
                      + " (IPointer/ptr-context #ptr {:context :test :id \"ROOT\"}) "
                      + " (get #ptr {:context :test :id \"ROOT\"} :id) "
                      + " (count #ptr {:context :test :id \"ROOT\"}) "
                      + " (find #ptr {:context :test :id \"ROOT\"} :id)]")
              .toString());
      assertEquals(
          "[:pointer/deref true \"ROOT\"]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (require 'std.lib.context.registry) "
                      + " (let [result (deref #ptr {:context :null :id \"ROOT\"})] "
                      + "  [(first result) (pointer? (second result)) (get (second result) :id)]))")
              .toString());
      assertEquals(
          "[:pointer/invoke true \"ROOT\" 1 2]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [result (#ptr {:context :null :id \"ROOT\"} 1 2)] "
                      + " [(first result) (pointer? (second result)) "
                      + "  (get (second result) :id) (nth result 2) (nth result 3)])")
              .toString());

      for (String source :
          new String[] {
            "#ptr {:id \"ROOT\"}",
            "#ptr {:context \"test\" :id \"ROOT\"}",
            "#ptr {:context :test \"id\" \"ROOT\"}"
          }) {
        assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, source));
      }
    }
  }

  @Test
  public void supportsVariadicDissocAndBoxedPromiseDeref() {
    try (Context context = context()) {
      assertEquals(
          "{:c 3}",
          context.eval(HaraLanguage.ID, "(dissoc {:a 1 :b 2 :c 3} :a :b)").toString());
      assertEquals(
          9L,
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (defn promised-value [] (std.foundation.promise/from 9)) "
                      + "(deref (promised-value)))")
              .asLong());
    }
  }

  @Test
  public void invokesTheFunctionValueHeldByAVar() {
    try (Context context = context()) {
      assertEquals(42, context.eval(HaraLanguage.ID, "((var +) 19 23)").asLong());
    }
  }

  @Test
  public void readsDataWithoutEvaluatingIt() {
    try (Context context = context()) {
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(= 12.5 (read-string \"12.5\"))")
              .asBoolean());
      assertEquals(
          12.5,
          context
              .eval(HaraLanguage.ID, "(double (read-string \"12.5\"))")
              .asDouble(),
          0.0);
      assertEquals(
          "{:value [1 (double 2.5)]}",
          context
              .eval(HaraLanguage.ID, "(pr-str (read-string \"{:value [1 2.5]}\"))")
              .asString());
    }
  }

  @Test
  public void exposesOrdinaryProtocolBackedCollectionFunctions() {
    try (Context context = context()) {
      assertEquals(3, context.eval(HaraLanguage.ID, "(count [1 2 3])").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(count (object \"a\" 1 \"b\" 2))").asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "(get {:a 1} :a)").asLong());
      assertEquals(7, context.eval(HaraLanguage.ID, "(get {} :missing 7)").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(nil? (get '(1) 1))").asBoolean());
      assertEquals(7, context.eval(HaraLanguage.ID, "(get [1] 2 7)").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(get (assoc {:a 1} :b 2) :b)").asLong());
      assertEquals(4, context.eval(HaraLanguage.ID, "(nth [3 4] 1)").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(count (conj [5] 6))").asLong());
      assertEquals(0, context.eval(HaraLanguage.ID, "(count (conj))").asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "(nth (conj 1) 0)").asLong());
      assertEquals(3, context.eval(HaraLanguage.ID, "(count (conj [] 1 2 3))").asLong());
      assertEquals(3, context.eval(HaraLanguage.ID, "(nth (conj [] 1 2 3) 2)").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(count (cons 0 '(1)))").asLong());
      assertEquals(0, context.eval(HaraLanguage.ID, "(count (empty [1 2]))").asLong());
      assertEquals(0, context.eval(HaraLanguage.ID, "(count nil)").asLong());
      assertEquals(7, context.eval(HaraLanguage.ID, "(get nil :missing 7)").asLong());
      assertEquals(
          "[:success 42 nil {:source :test}]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [result (std.native.Result/create :success 42 {:source :test})] "
                      + "[(:status result) (:data result) (:error result) (:context result)])")
              .toString());
      assertEquals(1, context.eval(HaraLanguage.ID, "(:a {:a 1})").asLong());
      assertEquals(7, context.eval(HaraLanguage.ID, "(:missing {} 7)").asLong());
      assertEquals(
          "[1 7 nil]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(let [key :a] (key {:a 1})) "
                      + " (let [key :missing] (key {} 7)) "
                      + " (let [key :missing] (key nil))]")
              .toString());
      assertTrue(context.eval(HaraLanguage.ID, "(empty nil)").isNull());
      assertEquals(
          ":ok",
          context.eval(HaraLanguage.ID, "(if ((fn [] nil)) :bad :ok)").toString());
      assertEquals(
          1,
          context
              .eval(HaraLanguage.ID, "(count (conj #{} ((fn [] nil))))")
              .asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "(count (conj nil 1))").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(count (set [1 1 2]))").asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "(count (cons 1 nil))").asLong());
    }
  }

  @Test
  public void rejectsArgumentsToNullaryCurrentSymbols() {
    try (Context context = context()) {
      PolyglotException error =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(current-symbols 1)"));
      assertTrue(error.getMessage().contains("expects 0 arguments"));
    }
  }

  @Test
  public void loadsTheLanguageLevelCoreBootstrapResource() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(HaraLanguage.ID, "(load-resource \"std/foundation.hal\") ((comp inc inc) 40)")
              .asLong());
      assertEquals(42, context.eval(HaraLanguage.ID, "((comp inc inc inc) 39)").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "((complement (fn [x] (= x 1))) 2)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(zero? 0)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(pos? 2)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(neg? -2)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(even? 4)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(odd? 5)").asBoolean());
      assertEquals(9, context.eval(HaraLanguage.ID, "((constantly 9) nil)").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(nil? nil)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(empty? [])").asBoolean());
      assertEquals(1, context.eval(HaraLanguage.ID, "(first [1 2])").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(second [1 2])").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(first (rest [1 2]))").asLong());
      assertEquals(42, context.eval(HaraLanguage.ID, "(get-in {:a {:b 42}} [:a :b])").asLong());
      assertEquals(
          42, context.eval(HaraLanguage.ID, "(get-in (assoc-in {} [:a :b] 42) [:a :b])").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(get (update {:a 1} :a inc) :a)").asLong());
      assertEquals(
          4,
          context
              .eval(HaraLanguage.ID, "(get-in (update-in {:a {:b 2}} [:a :b] + 2) [:a :b])")
              .asLong());
      assertEquals(3, context.eval(HaraLanguage.ID, "(last [1 2 3])").asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "(first (reverse [3 2 1]))").asLong());
      assertEquals(":a", context.eval(HaraLanguage.ID, "(first (keys {:a 1}))").toString());
      assertEquals(1, context.eval(HaraLanguage.ID, "(first (vals {:a 1}))").asLong());
      assertTrue(
          context.eval(HaraLanguage.ID, "(has? {:a nil} :a)").asBoolean());
      assertTrue(
          !context.eval(HaraLanguage.ID, "(has? {:a 1} :b)").asBoolean());
      assertEquals(2, context.eval(HaraLanguage.ID, "(get (dissoc {:a 1 :b 2} :a) :b)").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(nil? (get (dissoc {:a 1} :a) :a))").asBoolean());
      assertEquals(1, context.eval(HaraLanguage.ID, "(peek [1 2])").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(peek (pop '(1 2)))").asLong());
      assertEquals(0, context.eval(HaraLanguage.ID, "(iter-next (iter (range 3)))").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(iter-next (iter (range 2 4)))").asLong());
      assertEquals(10, context.eval(HaraLanguage.ID, "(iter-next (iter (iterate inc 10)))").asLong());
      assertEquals(7, context.eval(HaraLanguage.ID, "(iter-next (iter (repeat 2 7)))").asLong());
      assertEquals(
          5, context.eval(HaraLanguage.ID, "(iter-next (iter (repeatedly 1 (fn [] 5))))").asLong());
      assertEquals(
          1,
          context
              .eval(HaraLanguage.ID, "(first (take-while (fn [x] (< x 3)) [1 2 3]))")
              .asLong());
      assertEquals(
          3,
          context
              .eval(HaraLanguage.ID, "(first (drop-while (fn [x] (< x 3)) [1 2 3]))")
              .asLong());
      assertEquals(
          2,
          context.eval(HaraLanguage.ID, "(nth (first (partition-all 2 [1 2 3])) 1)").asLong());
      assertEquals(
          2, context.eval(HaraLanguage.ID, "(nth (first (partition 2 [1 2 3])) 1)").asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "(first (interpose 0 [1 2]))").asLong());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= (interpose 0 [1 2]) [1 0 2])")
              .asBoolean());
      assertEquals(
          1, context.eval(HaraLanguage.ID, "(first (interleave [1 2] [3 4]))").asLong());
      assertEquals(
          3,
          context
              .eval(HaraLanguage.ID, "(iter-next (Iter/iter-drop 1 (interleave [1 2] [3 4])))")
              .asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(not-empty [1])").hasArrayElements());
      assertTrue(context.eval(HaraLanguage.ID, "(not-empty [])").isNull());
      assertEquals(4, context.eval(HaraLanguage.ID, "(first (map inc [3]))").asLong());
      assertEquals(4, context.eval(HaraLanguage.ID, "(first (map + [1 2] [3 4]))").asLong());
      assertEquals(
          2,
          context.eval(HaraLanguage.ID, "(first (filter (fn [x] (= x 2)) [1 2 3]))").asLong());
      assertEquals(
          2, context.eval(HaraLanguage.ID, "(first (take 1 (drop 1 [1 2])))").asLong());
      assertEquals(
          2, context.eval(HaraLanguage.ID, "(first (mapcat (fn [x] [x x]) [2]))").asLong());
      assertEquals(
          2,
          context
              .eval(HaraLanguage.ID, "(first (keep (fn [x] (if (= x 2) x nil)) [1 2]))")
              .asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "(iter-next (iter (cycle [1 2])))").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(nth (first (zip [1] [2])) 1)").asLong());
      assertEquals(
          2, context.eval(HaraLanguage.ID, "(nth (first (partition-pair [1 2])) 1)").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(every? (fn [x] (> x 0)) [1 2])").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(any? (fn [x] (= x 2)) [1 2])").asBoolean());
      assertEquals(6, context.eval(HaraLanguage.ID, "(reduce + [1 2 3])").asLong());
      assertEquals(16, context.eval(HaraLanguage.ID, "(reduce + 10 [1 2 3])").asLong());
    }
  }

  @Test
  public void supportsLazySeqBoundariesAndSourceAwareTransforms() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(load-resource \"std/foundation.hal\")");
      assertTrue(context.eval(HaraLanguage.ID, "(vector? (map inc [1 2 3]))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(vector? ((map inc) [1 2 3]))").asBoolean());
      assertTrue(
          context.eval(HaraLanguage.ID, "(seq? ((map inc) (seq [1 2 3])))").asBoolean());
      assertTrue(
          context.eval(HaraLanguage.ID, "(iter? ((map inc) (iter [1 2 3])))").asBoolean());
      assertEquals(1, context.eval(HaraLanguage.ID, "(first [1 2])").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(last [1 2])").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(first (map inc [1 2 3]))").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(first ((map inc) [1 2 3]))").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(first ((map inc) (seq [1 2 3])))").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(first ((map inc) [1 2 3]))").asLong());
      assertEquals(
          3,
          context
              .eval(HaraLanguage.ID, "(first ((comp (map inc) (map inc)) [1 2 3]))")
              .asLong());
      assertEquals(
          3,
          context.eval(HaraLanguage.ID, "(first ((comp (map inc) (map inc)) [1 2 3]))").asLong());
      assertEquals(
          2,
          context
              .eval(
                  HaraLanguage.ID,
                  "(->> (iterate inc 0) (drop 1) "
                      + "(take-while (fn [value] (< value 5))) "
                      + "(filter (fn [value] (= 0 (mod value 2)))) first)")
              .asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(nil? (rest [1]))").asBoolean());
      assertEquals(
          "[true false 1 1 2 [1 2 3] [1 2 3]]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [xs (seq [1 2 3])] "
                      + "[(seq? xs) (iter? xs) (first xs) (first xs) "
                      + " (first (rest xs)) (vec xs) (vec xs)])")
              .toString());
      assertEquals(
          "[1 1 2 2]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [xs (seq [1 2 3]) left (iter xs) right (iter xs)] "
                      + "[(iter-next left) (iter-next right) "
                      + " (iter-next left) (iter-next right)])")
              .toString());
      assertEquals(
          "[1 1]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [calls (atom 0) xs (seq (Iter/iter-map "
                      + "(fn [value] (do (swap! calls inc) value)) [1 2]))] "
                      + "[(first xs) (do (first xs) (deref calls))])")
              .toString());
      assertEquals(
          "[[0 1 2 3 4] [0 1 2 3 4]]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [calls (atom []) values (vec (take 5 (iterate "
                      + "(fn [value] (do (swap! calls conj value) (inc value))) 0)))] "
                      + "[values (deref calls)])")
              .toString());
      assertEquals(
          "(0 1 2 3 4 5 6 7 8 9 ...)",
          context.eval(HaraLanguage.ID, "(seq (Iter/iter-range 20))").toString());
    }
  }

  @Test
  public void sequentialLookupRejectsInvalidIndicesWithoutHostFailures() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(load-resource \"std/foundation.hal\")");
      PolyglotException error =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(get [0 1 2 3 4 5 6 7 8] nil)"));
      assertTrue(error.getMessage().contains("sequential lookup expects a non-negative integer"));
    }
  }

  @Test
  public void requiresPackagedCoreBootstrapAsAClasspathModule() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(HaraLanguage.ID, "(require \"std/foundation.hal\") ((comp inc inc) 40)")
              .asLong());
      assertEquals(42, context.eval(HaraLanguage.ID, "((comp inc inc inc) 39)").asLong());
      assertEquals(
          1, context.eval(HaraLanguage.ID, "(module-revision \"std/foundation.hal\")").asLong());
      context.eval(HaraLanguage.ID, "(require \"std/foundation.hal\" {:reload true})");
      assertEquals(
          2,
          context
              .eval(HaraLanguage.ID, "(module-revision \"classpath:std/foundation.hal\")")
              .asLong());
    }
  }

  @Test
  public void dispatchesDefmultiAndDefmethodByArbitraryValues() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(defmulti kind (fn [x] (if (= x 1) :one :other)))");
      context.eval(HaraLanguage.ID, "(defmethod kind :one [x] \"one\")");
      context.eval(HaraLanguage.ID, "(defmethod kind :default [x] \"other\")");
      assertEquals("one", context.eval(HaraLanguage.ID, "(kind 1)").asString());
      assertEquals("other", context.eval(HaraLanguage.ID, "(kind 2)").asString());
      context.eval(HaraLanguage.ID, "(defmethod kind :one [x] \"updated\")");
      assertEquals("updated", context.eval(HaraLanguage.ID, "(kind 1)").asString());
      assertEquals("other", context.eval(HaraLanguage.ID, "(apply kind 2 [])").asString());
    }
  }

  @Test
  public void supportsMutuallyRecursiveLetfnBindings() {
    try (Context context = context()) {
      assertEquals(
          120,
          context
              .eval(
                  HaraLanguage.ID,
                  "(letfn [(fact [n] (if (= n 0) 1 (* n (fact (- n 1)))))] (fact 5))")
              .asLong());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(letfn [(even? [n] (if (= n 0) true (odd? (- n 1)))) "
                      + "(odd? [n] (if (= n 0) false (even? (- n 1))))] (even? 10))")
              .asBoolean());
    }
  }

  @Test
  public void evaluatesCondClausesInOrder() {
    try (Context context = context()) {
      assertEquals(
          42, context.eval(HaraLanguage.ID, "(cond (= 1 2) 0 (= 2 2) 42 :else 99)").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(cond false nil :else true)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(cond false nil)").isNull());
    }
  }

  @Test
  public void expandsThreadFirstAndThreadLastForms() {
    try (Context context = context()) {
      assertEquals(15, context.eval(HaraLanguage.ID, "(-> 3 (+ 2) (* 3))").asLong());
      assertEquals(15, context.eval(HaraLanguage.ID, "(->> 3 (+ 2) (* 3))").asLong());
      assertEquals(6, context.eval(HaraLanguage.ID, "(-> 1 (+ 2 3))").asLong());
      assertEquals(6, context.eval(HaraLanguage.ID, "(->> 1 (+ 2 3))").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(let [% 1] (+ % %))").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(-> 1 (+ % %))").asLong());
      assertEquals(6, context.eval(HaraLanguage.ID, "(->> 3 (+ % %))").asLong());
      assertEquals("[1 %]", context.eval(HaraLanguage.ID, "(-> 1 (vector '%))").toString());
    }
  }

  @Test
  public void supportsVariadicMacrosAndSyntaxQuoteSplicing() {
    try (Context context = context()) {
      assertEquals(
          3,
          context
              .eval(
                  HaraLanguage.ID, "(do (defmacro do-all [& forms] `(do ~@forms)) (do-all 1 2 3))")
              .asLong());
      assertEquals(
          9,
          context
              .eval(
                  HaraLanguage.ID, "(do (defmacro wrap [x & forms] `(do ~x ~@forms)) (wrap 4 5 9))")
              .asLong());
    }
  }

  @Test
  public void keepsSpecialFormsUnshadowableByMacros() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(defmacro do [& forms] 99)");
      assertEquals(3, context.eval(HaraLanguage.ID, "(do 1 2 3)").asLong());
      assertEquals("(do 1 2 3)", context.eval(HaraLanguage.ID, "(macroexpand (quote (do 1 2 3)))").toString());
      assertEquals(99, context.eval(HaraLanguage.ID, "(user/do 1 2 3)").asLong());
    }
  }

  @Test
  public void defmacroPreservesDocumentationAttributesAndArglists() {
    try (Context context = context()) {
      var metadata =
          context.eval(
              HaraLanguage.ID,
              "(do"
                  + " (defmacro documented \"macro docs\" {:added \"1.2\"} [value]"
                  + "   `(identity ~value))"
                  + " (let [m (meta (var documented))]"
                  + "   [(:doc m) (:added m) (:arglists m) (:macro m)]))");
      assertEquals("macro docs", metadata.getArrayElement(0).asString());
      assertEquals("1.2", metadata.getArrayElement(1).asString());
      assertEquals("[[value]]", metadata.getArrayElement(2).toString());
      assertTrue(metadata.getArrayElement(3).asBoolean());
      assertEquals(9L, context.eval(HaraLanguage.ID, "(documented 9)").asLong());
    }
  }

  @Test
  public void supportsDeclarationsAndPrivateDefinitions() {
    try (Context context = context()) {
      assertEquals(
          42,
          context.eval(HaraLanguage.ID, "(do (declare answer) (def answer 42) answer)").asLong());
      assertEquals(
          7,
          context
              .eval(HaraLanguage.ID, "(do (defn- private-answer [] 7) (private-answer))")
              .asLong());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(ILookup/lookup (IObjType/meta (var private-answer)) :private)")
              .asBoolean());
    }
  }

  @Test
  public void exposesMacroexpandFormsForTheRetainedRepl() {
    try (Context context = context()) {
      assertEquals(
          "(if false nil 42)",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (defmacro unless [test body] `(if ~test nil ~body)) "
                      + "(macroexpand-1 '(unless false 42)))")
              .toString());
      assertEquals(
          "(if true nil 7)",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (defmacro inner [x] `(if true nil ~x)) "
                      + "(defmacro outer [x] `(inner ~x)) "
                      + "(macroexpand '(outer 7)))")
              .toString());
    }
  }

  @Test
  public void evaluatesSpecializedArithmeticOperations() {
    try (Context context = context()) {
      assertEquals(0, context.eval(HaraLanguage.ID, "(+)").asLong());
      assertEquals(6, context.eval(HaraLanguage.ID, "(+ 1 2 3)").asLong());
      assertEquals(7, context.eval(HaraLanguage.ID, "(- 10 3)").asLong());
      assertEquals(-10, context.eval(HaraLanguage.ID, "(- 10)").asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "(*)").asLong());
      assertEquals(42, context.eval(HaraLanguage.ID, "(* 6 7)").asLong());
      assertEquals(24, context.eval(HaraLanguage.ID, "(* 2 3 4)").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(/ 5 2)").asLong());
      assertEquals(0, context.eval(HaraLanguage.ID, "(/ 2)").asLong());
      assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(/)"));
    }
  }

  @Test
  public void evaluatesNumericComparisonsEqualityAndRemainder() {
    try (Context context = context()) {
      assertTrue(context.eval(HaraLanguage.ID, "(< 1 2)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(<= 2 2)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(> 3 2)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(>= 3 3)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(= 1 1.0)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(not= 1 2)").asBoolean());
      assertEquals(1, context.eval(HaraLanguage.ID, "(mod 7 3)").asLong());
      assertEquals(-1, context.eval(HaraLanguage.ID, "(mod -7 3)").asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "(mod 7 -3)").asLong());
      assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(% 7 3)"));
      assertTrue(context.eval(HaraLanguage.ID, "(< 1 2 3)").asBoolean());
      assertTrue(!context.eval(HaraLanguage.ID, "(< 1 3 2)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(< 1 1.1)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(= 1 1.0)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(= 1 1 1)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(< 1.2 1.3)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(< 1 2)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(apply < [1 2 3])").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(reduce < [1 2])").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(apply = [4 4])").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(reduce not= [4 5])").asBoolean());
    }
  }

  @Test
  public void numericPromotionAndDivisionErrorsAreExplicit() {
    try (Context context = context()) {
      assertEquals(
          new java.math.BigInteger("9223372036854775808"),
          context.eval(HaraLanguage.ID, "(+ 9223372036854775807 1)")
              .as(java.math.BigInteger.class));
      PolyglotException divideByZero =
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(/ 1 0)"));
      assertTrue(divideByZero.getMessage().contains("Divide by zero"));
      PolyglotException remainderByZero =
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(mod 1 0)"));
      assertTrue(remainderByZero.getMessage().contains("Divide by zero"));
    }
  }

  @Test
  public void ratioLiteralsAreRejectedAsAnExplicitUnsupportedBoundary() {
    try (Context context = context()) {
      PolyglotException ratio =
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "1/2"));
      assertTrue(ratio.getMessage(), ratio.getMessage().contains("Ratios are not supported"));
    }
  }

  @Test
  public void numericValuesRejectNonFiniteResults() {
    try (Context context = context()) {
      assertTrue(context.eval(HaraLanguage.ID, "(= -0.0 0.0)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(= 1.0 1.00)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(long? 1)").asBoolean());
      assertFalse(context.eval(HaraLanguage.ID, "(long? 1.0)").asBoolean());
      assertFalse(
          context.eval(HaraLanguage.ID, "(long? 9223372036854775808)").asBoolean());
      assertTrue(
          context.eval(HaraLanguage.ID, "(bigint? 9223372036854775808)").asBoolean());
      assertFalse(context.eval(HaraLanguage.ID, "(bigint? 1)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(integer? 1)").asBoolean());
      assertTrue(
          context.eval(HaraLanguage.ID, "(integer? 9223372036854775808)").asBoolean());
      assertFalse(context.eval(HaraLanguage.ID, "(integer? 1.0)").asBoolean());
      for (String source :
          new String[] {
            "##NaN",
            "##Inf",
            "##-Inf",
            "1e309",
            "(sqrt -1)",
            "(exp 10000)",
            "(* 1.0e308 1.0e308)",
            "(std.native.Num/parse-double \"Infinity\")"
          }) {
        assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, source));
      }
      assertTrue(context.eval(HaraLanguage.ID, "(not false)").asBoolean());
      assertTrue(!context.eval(HaraLanguage.ID, "(not true)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(not nil)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(and true 7)").asLong() == 7);
      assertTrue(context.eval(HaraLanguage.ID, "(or false 8)").asLong() == 8);
      assertTrue(context.eval(HaraLanguage.ID, "(and false (/ 1 0))").asBoolean() == false);
      assertTrue(context.eval(HaraLanguage.ID, "(or true (/ 1 0))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(when true 4 5)").asLong() == 5);
      assertTrue(context.eval(HaraLanguage.ID, "(when false (/ 1 0))").isNull());
      assertTrue(context.eval(HaraLanguage.ID, "(when-not false 6)").asLong() == 6);
    }
  }

  @Test
  public void constructsBytesWithTheOrdinaryBytesForm() {
    try (Context context = context()) {
      Value bytes = context.eval(HaraLanguage.ID, "(bytes 1 2 -3)");
      assertTrue(bytes.hasArrayElements());
      assertEquals(3, bytes.getArraySize());
      assertEquals(1, bytes.getArrayElement(0).asLong());
      assertEquals(2, bytes.getArrayElement(1).asLong());
      assertEquals(-3, bytes.getArrayElement(2).asLong());
      assertEquals(
          3, context.eval(HaraLanguage.ID, "(ICount/count (bytes 1 2 -3))").asLong());
      assertEquals(
          2, context.eval(HaraLanguage.ID, "(INth/nth (bytes 1 2 -3) 1)").asLong());
      assertEquals(
          7,
          context
              .eval(HaraLanguage.ID, "(ILookup/lookup (bytes 1 2 -3) 8 7)")
              .asLong());
      bytes.setArrayElement(0, 9);
      assertEquals(9, bytes.getArrayElement(0).asLong());
    }
  }

  @Test
  public void constructsAndMutatesExplicitMarkerValues() {
    try (Context context = context()) {
      Value array = context.eval(HaraLanguage.ID, "(array 1 2)");
      assertTrue(array.hasArrayElements());
      array.setArrayElement(1, 7);
      assertEquals(7, array.getArrayElement(1).asLong());

      Value object = context.eval(HaraLanguage.ID, "(object \"answer\" 41)");
      assertTrue(object.hasHashEntries());
      object.putHashEntry("answer", 42);
      assertEquals(42, object.getHashValue("answer").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(count (array 1 2))").asLong());
      assertEquals(
          7,
          context
              .eval(HaraLanguage.ID, "(let [a (array 1 2)] (Arr/set a 1 7) (Arr/get a 1))")
              .asLong());
      assertEquals(
          1,
          context
              .eval(HaraLanguage.ID, "(let [a (array 1 2)] (Arr/remove a 0) (count a))")
              .asLong());
      assertEquals(
          3,
          context
              .eval(HaraLanguage.ID, "(let [a (array 1 2)] (Arr/push-last a 3) (count a))")
              .asLong());
      assertEquals(
          7,
          context
              .eval(HaraLanguage.ID, "(let [a (array 1 2)] (Arr/insert a 1 7) (Arr/get a 1))")
              .asLong());
      assertEquals(
          2, context.eval(HaraLanguage.ID, "(count (Arr/slice (array 1 2 3) 1 3))").asLong());
      assertEquals(
          2, context.eval(HaraLanguage.ID, "(Arr/get (Arr/clone (array 1 2)) 1)").asLong());
    }
  }

  @Test
  public void convertsBetweenNumericRepresentationsExplicitly() {
    try (Context context = context()) {
      assertEquals(1, context.eval(HaraLanguage.ID, "(long 1.0)").asLong());
      assertEquals(-1, context.eval(HaraLanguage.ID, "(long -1.0)").asLong());
      assertEquals(2.0, context.eval(HaraLanguage.ID, "(double 2)").asDouble(), 0.0);
      assertEquals(1, context.eval(HaraLanguage.ID, "(long 1.9)").asLong());
      assertEquals(-1, context.eval(HaraLanguage.ID, "(long -1.9)").asLong());
      assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(long \"1\")"));
      assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "1N"));
      assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "1M"));
    }
  }

  @Test
  public void evaluatesControlFlowAndSequentialBodies() {
    try (Context context = context()) {
      assertEquals(2, context.eval(HaraLanguage.ID, "(if false 1 2)").asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "(if 0 1 2)").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(if nil 1)").isNull());
      assertEquals(3, context.eval(HaraLanguage.ID, "(do 1 2 3)").asLong());
    }
  }

  @Test
  public void evaluatesLoopAndTailRecur() {
    try (Context context = context()) {
      assertEquals(
          42,
          context.eval(HaraLanguage.ID, "(loop [value 41] (if value (recur nil) 42))").asLong());
    }
  }

  @Test
  public void rejectsRecurOutsideLoopAndNonTailRecur() {
    try (Context context = context()) {
      PolyglotException outside =
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(recur)"));
      assertTrue(outside.getMessage().contains("outside loop"));

      PolyglotException nonTail =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(loop [value true] (+ (recur nil) 1))"));
      assertTrue(nonTail.getMessage().contains("tail position"));
    }
  }

  @Test
  public void evaluatesNestedLoopsWithDistinctRecurTargets() {
    try (Context context = context()) {
      assertEquals(
          18,
          context
              .eval(
                  HaraLanguage.ID,
                  "(loop [i 0 acc 0] "
                      + "(if (< i 3) "
                      + "  (recur (+ i 1) "
                      + "    (+ acc (loop [j 0 inner 0] "
                      + "             (if (< j 4) (recur (+ j 1) (+ inner j)) inner)))) "
                      + "  acc))")
              .asLong());
    }
  }

  @Test
  public void evaluatesRecurInIfDoAndLetTailPositions() {
    try (Context context = context()) {
      assertEquals(
          2, context.eval(HaraLanguage.ID, "(loop [i 0] (if (< i 2) (recur (+ i 1)) i))").asLong());
      assertEquals(
          2,
          context
              .eval(HaraLanguage.ID, "(loop [i 0] (do 42 (if (< i 2) (recur (+ i 1)) i)))")
              .asLong());
      assertEquals(
          3,
          context
              .eval(HaraLanguage.ID, "(loop [i 0] (let [x (+ i 1)] (if (< x 3) (recur x) x)))")
              .asLong());
    }
  }

  @Test
  public void rejectsRecurArityMismatch() {
    try (Context context = context()) {
      PolyglotException mismatch =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(loop [left 1 right 2] (recur 3))"));
      assertTrue(mismatch.getMessage().contains("recur expects 2 arguments"));
    }
  }

  @Test
  public void recurValuesObserveCurrentBindingsSimultaneously() {
    try (Context context = context()) {
      // Every recurrence value must be evaluated before any binding is updated:
      // (recur (+ x y) (+ y 1)) reads the old x and y on every iteration.
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= [3 3] (loop [x 1 y 2] (if (< x 3) (recur (+ x y) (+ y 1)) [x y])))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= [6 4] (loop [a 1 b 2 n 0] "
                      + "(if (< n 2) (recur (+ a b) (+ b 1) (+ n 1)) [a b])))")
              .asBoolean());
    }
  }

  @Test
  public void rejectsRecurInsideTryAsNonTail() {
    try (Context context = context()) {
      PolyglotException nonTail =
          assertThrows(
              PolyglotException.class,
              () ->
                  context.eval(
                      HaraLanguage.ID, "(loop [i 0] (try (recur (+ i 1)) (finally 42)))"));
      assertTrue(nonTail.getMessage().contains("tail position"));
    }
  }

  @Test
  public void guestCatchDoesNotInterceptRecurrence() {
    try (Context context = context()) {
      assertEquals(
          5,
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (loop [i 0] (if (< i 5) (recur (+ i 1)) i)) (catch Throwable t 99))")
              .asLong());
    }
  }

  @Test
  public void closedLambdasHaveNoCapturesAndAreReused() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(defn mk-closed [] (fn [x] (+ x 1)))");
      assertEquals(2, context.eval(HaraLanguage.ID, "((mk-closed) 1)").asLong());
      // A closure-free literal yields one immutable function value on every execution.
      assertTrue(context.eval(HaraLanguage.ID, "(= (mk-closed) (mk-closed))").asBoolean());
    }
  }

  @Test
  public void closuresCaptureOuterLexicalBindings() {
    try (Context context = context()) {
      assertEquals(
          11, context.eval(HaraLanguage.ID, "(((fn [a b] (fn [x] (+ a x))) 10 20) 1)").asLong());
      assertEquals(
          31,
          context
              .eval(HaraLanguage.ID, "(((fn [a b] (fn [x] (+ a (+ b x)))) 10 20) 1)")
              .asLong());
      assertEquals(
          6, context.eval(HaraLanguage.ID, "(let [x 1 y 2 f (fn [z] (+ x (+ y z)))] (f 3))").asLong());
    }
  }

  @Test
  public void lexicalShadowingResolvesNearestBinding() {
    try (Context context = context()) {
      assertEquals(11, context.eval(HaraLanguage.ID, "(let [x 1] ((fn [x] (+ x 1)) 10))").asLong());
      assertEquals(5, context.eval(HaraLanguage.ID, "((fn [x] ((fn [x] x) 5)) 1)").asLong());
      assertEquals(
          2, context.eval(HaraLanguage.ID, "(let [x 1 f (let [x 2] (fn [] x))] (f))").asLong());
    }
  }

  @Test
  public void nestedClosureCapturesGrandparentLocalTransitively() {
    try (Context context = context()) {
      assertEquals(
          6,
          context
              .eval(HaraLanguage.ID, "((((fn [x] (fn [y] (fn [z] (+ x (+ y z))))) 1) 2) 3)")
              .asLong());
      // The middle function reads only y; it must capture x solely for its nested child.
      assertEquals(
          3,
          context.eval(HaraLanguage.ID, "(((fn [x] (fn [y] ((fn [z] (+ x z)) y))) 1) 2)").asLong());
    }
  }

  @Test
  public void lexicalBindingInGrandparentScopeBlocksMacroExpansion() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(defmacro shadowed-macro [x] 99)");
      assertEquals(
          99, context.eval(HaraLanguage.ID, "(shadowed-macro 1)").asLong());
      assertEquals(
          3,
          context
              .eval(
                  HaraLanguage.ID,
                  "((fn [shadowed-macro] ((fn [] (shadowed-macro 2)))) (fn [x] (+ x 1)))")
              .asLong());
    }
  }

  @Test
  public void closuresShareStableCallTargetAcrossWrappers() {
    try (Context context = context()) {
      assertEquals(
          23,
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [adder (fn [n] (fn [x] (+ x n)))] (+ ((adder 1) 10) ((adder 2) 10)))")
              .asLong());
    }
  }

  @Test
  public void polymorphicCallSiteFallsBackToIndirectCalls() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(defn apply2 [f x] (f x))");
      assertEquals(
          31,
          context
              .eval(
                  HaraLanguage.ID,
                  "(+ (apply2 (fn [x] (+ x 1)) 10) (apply2 (fn [x] (* x 2)) 10))")
              .asLong());
    }
  }

  @Test
  public void globalVarLookupSeesRedefinitionThroughFunctions() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(def gv 1)");
      context.eval(HaraLanguage.ID, "(defn read-gv [] gv)");
      assertEquals(1, context.eval(HaraLanguage.ID, "(read-gv)").asLong());
      context.eval(HaraLanguage.ID, "(def gv 9)");
      assertEquals(9, context.eval(HaraLanguage.ID, "(read-gv)").asLong());
    }
  }

  @Test
  public void dynamicVarsRemainVarLookupsInsideClosures() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(def ^:dynamic *dv* 1)");
      context.eval(HaraLanguage.ID, "(defn make-reader [] (fn [] *dv*))");
      assertEquals(1, context.eval(HaraLanguage.ID, "((make-reader))").asLong());
      assertEquals(
          42, context.eval(HaraLanguage.ID, "(binding [*dv* 42] ((make-reader)))").asLong());
    }
  }

  @Test
  public void finallyRunsOnceWhenEnclosingLoopCompletes() {
    try (Context context = context()) {
      assertEquals(
          3,
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (loop [i 0] (if (< i 3) (recur (+ i 1)) i)) "
                      + "(finally (def loop-finally-runs 1)))")
              .asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "loop-finally-runs").asLong());
    }
  }

  @Test
  public void evaluatesThrowCatchAndFinally() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (throw (ex :test/value {:value 41})) "
                      + "(catch Exception error (+ (:value (ex-data error)) 1)) "
                      + "(finally (def cleaned true)))")
              .asLong());
      assertTrue(context.eval(HaraLanguage.ID, "cleaned").asBoolean());
    }
  }

  @Test
  public void supportsOrderedGuestErrorCodeCatchClauses() {
    try (Context context = context()) {
      assertEquals(
          7,
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (throw (ex :problem/value {:ex/message \"problem\" :value 7})) "
                      + "(catch :problem/other error 0) "
                      + "(catch :problem/value error (:value (ex-data error))))")
              .asLong());
      PolyglotException unmatched =
          assertThrows(
              PolyglotException.class,
              () ->
                  context.eval(
                      HaraLanguage.ID,
                      "(try (throw (ex :problem/value {:ex/message \"problem\"})) "
                          + "(catch :problem/other error error))"));
      assertTrue(unmatched.isGuestException());
    }
  }

  @Test
  public void storesLexicalBindingsInFrames() {
    try (Context context = context()) {
      assertEquals(5, context.eval(HaraLanguage.ID, "(let [x 2 y 3] (+ x y))").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(let [x 1] (let [x 2] x))").asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "(let [x 1 y x] y)").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(let [x 1 x (+ x 1)] x)").asLong());
    }
  }

  @Test
  public void persistsDefinitionsAcrossEvaluations() {
    try (Context context = context()) {
      assertEquals(
          "#'user/player", context.eval(HaraLanguage.ID, "(def player 1)").toString());
      assertEquals(
          "#'user/player", context.eval(HaraLanguage.ID, "(def player 2)").toString());
      assertEquals(2, context.eval(HaraLanguage.ID, "player").asLong());
      assertEquals(
          "#'user/answer", context.eval(HaraLanguage.ID, "(def answer 41)").toString());
      assertEquals(42, context.eval(HaraLanguage.ID, "(+ answer 1)").asLong());
      context.eval(HaraLanguage.ID, "(def answer 42)");
      assertEquals(42, context.eval(HaraLanguage.ID, "answer").asLong());
    }
  }

  @Test
  public void anonymousNamespaceReusesTheCurrentSessionNamespace() {
    try (Context context = context()) {
      assertTrue(context.eval(HaraLanguage.ID, "(nil? (ns+))").asBoolean());
      assertEquals(
          "#'user/player", context.eval(HaraLanguage.ID, "(ns+) (def player 1)").toString());
      PolyglotException error =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(ns+ public.name)"));
      assertTrue(error.getMessage().contains("does not accept a namespace name"));
    }
  }

  @Test
  public void loadsHaraSourceIntoTheCurrentContext() throws Exception {
    try (Context context = context()) {
      assertEquals(
          42,
          context.eval(HaraLanguage.ID, "(load-string \"(def loaded 41)\") (+ loaded 1)").asLong());

      Path file = Files.createTempFile("hara-core-language-", ".hal");
      try {
        Files.writeString(file, "(def from-file 42)");
        String path = file.toString().replace("\\", "\\\\").replace("\"", "\\\"");
        context.eval(HaraLanguage.ID, "(load-file \"" + path + "\")");
        assertEquals(42, context.eval(HaraLanguage.ID, "from-file").asLong());
      } finally {
        Files.deleteIfExists(file);
      }
    }
  }

  @Test
  public void loadsPackagedHaraResourcesTransactionally() {
    try (Context context = context()) {
      assertEquals(
          42, context.eval(HaraLanguage.ID, "(load-resource \"hara/core-language-resource.hal\")").asLong());
      assertEquals(42, context.eval(HaraLanguage.ID, "core-language-resource-answer").asLong());
      assertTrue(
          assertThrows(
                  PolyglotException.class,
                  () -> context.eval(HaraLanguage.ID, "(load-resource \"hara/missing.hal\")"))
              .getMessage()
              .contains("Unable to find Hara resource"));
    }
  }

  @Test
  public void failedModuleEvaluationRollsBackVarsMacrosAndNamespace() throws Exception {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(def stable 7)");
      PolyglotException stringFailure =
          assertThrows(
              PolyglotException.class,
              () ->
                  context.eval(
                      HaraLanguage.ID,
                      "(load-string \"(ns transient) (def leaked 9) (throw :failed)\")"));
      assertTrue(stringFailure.getMessage().contains("Unable to evaluate Hara source"));
      assertEquals(7, context.eval(HaraLanguage.ID, "stable").asLong());
      assertTrue(
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "leaked"))
              .getMessage()
              .contains("Unbound symbol"));

      Path file = Files.createTempFile("hara-core-language-failing-", ".hal");
      try {
        Files.writeString(file, "(def file-leaked 10) (throw :failed-file)");
        String path = file.toString().replace("\\", "\\\\").replace("\"", "\\\"");
        assertThrows(
            PolyglotException.class,
            () -> context.eval(HaraLanguage.ID, "(load-file \"" + path + "\")"));
        assertTrue(
            assertThrows(
                    PolyglotException.class, () -> context.eval(HaraLanguage.ID, "file-leaked"))
                .getMessage()
                .contains("Unbound symbol"));
      } finally {
        Files.deleteIfExists(file);
      }
    }
  }

  @Test
  public void requireCachesCanonicalModulesAndLoadFileIncrementsRevision() throws Exception {
    try (Context context = context()) {
      Path file = Files.createTempFile("hara-core-language-module-", ".hal");
      try {
        Files.writeString(file, "(def module-answer 41)");
        String path = file.toString().replace("\\", "\\\\").replace("\"", "\\\"");
        context.eval(HaraLanguage.ID, "(require \"" + path + "\")");
        assertEquals(41, context.eval(HaraLanguage.ID, "module-answer").asLong());
        assertEquals(
            1, context.eval(HaraLanguage.ID, "(module-revision \"" + path + "\")").asLong());

        Files.writeString(file, "(def module-answer 42)");
        context.eval(HaraLanguage.ID, "(require \"" + path + "\")");
        assertEquals(41, context.eval(HaraLanguage.ID, "module-answer").asLong());
        assertEquals(
            1, context.eval(HaraLanguage.ID, "(module-revision \"" + path + "\")").asLong());

        context.eval(HaraLanguage.ID, "(load-file \"" + path + "\")");
        assertEquals(42, context.eval(HaraLanguage.ID, "module-answer").asLong());
        assertEquals(
            2, context.eval(HaraLanguage.ID, "(module-revision \"" + path + "\")").asLong());
        Files.writeString(file, "(def module-answer 43)");
        context.eval(HaraLanguage.ID, "(require \"" + path + "\" {:reload true})");
        assertEquals(43, context.eval(HaraLanguage.ID, "module-answer").asLong());
        assertEquals(
            3, context.eval(HaraLanguage.ID, "(module-revision \"" + path + "\")").asLong());
      } finally {
        Files.deleteIfExists(file);
      }
    }
  }

  @Test
  public void requirePreservesCallerNamespaceAndSupportsAliases() throws Exception {
    try (Context context = context()) {
      Path file = Files.createTempFile("hara-core-language-alias-", ".hal");
      try {
        Files.writeString(file, "(ns library) (def answer 42)");
        String path = file.toString().replace("\\", "\\\\").replace("\"", "\\\"");
        assertEquals(
            42,
            context
                .eval(HaraLanguage.ID, "(require \"" + path + "\" {:as 'lib}) lib/answer")
                .asLong());
        assertEquals(
            1, context.eval(HaraLanguage.ID, "(module-revision \"" + path + "\")").asLong());
        context.eval(HaraLanguage.ID, "(def caller-value 7)");
        assertEquals(7, context.eval(HaraLanguage.ID, "caller-value").asLong());
      } finally {
        Files.deleteIfExists(file);
      }
    }
  }

  @Test
  public void requireSupportsSelectiveLiveReferences() throws Exception {
    try (Context context = context()) {
      Path file = Files.createTempFile("hara-core-language-refer-", ".hal");
      try {
        Files.writeString(file, "(ns library) (def answer 41) (def other 7)");
        String path = file.toString().replace("\\", "\\\\").replace("\"", "\\\"");
        context.eval(HaraLanguage.ID, "(require \"" + path + "\" {:refer [answer]})");
        assertEquals(42, context.eval(HaraLanguage.ID, "(set! library/answer 42) answer").asLong());
        assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "other"));
      } finally {
        Files.deleteIfExists(file);
      }
    }
  }

  @Test
  public void requireSupportsSelectiveMacroReferences() throws Exception {
    try (Context context = context()) {
      Path file = Files.createTempFile("hara-core-language-macro-refer-", ".hal");
      try {
        Files.writeString(
            file, "(ns library-macros) (defmacro unless [test body] `(if ~test nil ~body))");
        String path = file.toString().replace("\\", "\\\\").replace("\"", "\\\"");
        context.eval(HaraLanguage.ID, "(require \"" + path + "\" {:refer-macros [unless]})");
        assertEquals(42, context.eval(HaraLanguage.ID, "(unless false 42)").asLong());
      } finally {
        Files.deleteIfExists(file);
      }
    }
  }

  @Test
  public void reloadingARequiredMacroModuleRefreshesNewCompilations() throws Exception {
    try (Context context = context()) {
      Path file = Files.createTempFile("hara-core-language-macro-reload-", ".hal");
      try {
        Files.writeString(file, "(ns reload-macros) (defmacro answer [] 41)");
        String path = file.toString().replace("\\", "\\\\").replace("\"", "\\\"");
        context.eval(HaraLanguage.ID, "(require \"" + path + "\" {:refer-macros [answer]})");
        assertEquals(41, context.eval(HaraLanguage.ID, "(answer)").asLong());
        Files.writeString(file, "(ns reload-macros) (defmacro answer [] 42)");
        context.eval(
            HaraLanguage.ID, "(require \"" + path + "\" {:reload true :refer-macros [answer]})");
        assertEquals("42", context.eval(HaraLanguage.ID, "(macroexpand '(answer))").toString());
        // Existing Truffle call targets are immutable; a newly compiled source sees the reload.
        assertEquals(42, context.eval(HaraLanguage.ID, "(answer )").asLong());
      } finally {
        Files.deleteIfExists(file);
      }
    }
  }

  @Test
  public void requireRejectsCyclesAndRollsBackPartialModules() throws Exception {
    try (Context context = context()) {
      Path directory = Files.createTempDirectory("hara-core-language-cycle-");
      Path first = directory.resolve("first.hal");
      Path second = directory.resolve("second.hal");
      try {
        String firstPath = first.toString().replace("\\", "\\\\").replace("\"", "\\\"");
        String secondPath = second.toString().replace("\\", "\\\\").replace("\"", "\\\"");
        Files.writeString(first, "(def first-value 1) (require \"" + secondPath + "\")");
        Files.writeString(second, "(def second-value 2) (require \"" + firstPath + "\")");
        PolyglotException cycle =
            assertThrows(
                PolyglotException.class,
                () -> context.eval(HaraLanguage.ID, "(require \"" + firstPath + "\")"));
        assertTrue(cycle.getMessage().contains("Cyclic module require"));
        assertTrue(
            assertThrows(
                    PolyglotException.class, () -> context.eval(HaraLanguage.ID, "first-value"))
                .getMessage()
                .contains("Unbound symbol"));
        assertTrue(
            assertThrows(
                    PolyglotException.class, () -> context.eval(HaraLanguage.ID, "second-value"))
                .getMessage()
                .contains("Unbound symbol"));
      } finally {
        Files.deleteIfExists(first);
        Files.deleteIfExists(second);
        Files.deleteIfExists(directory);
      }
    }
  }

  @Test
  public void requireRecordsDeterministicModuleDependencies() throws Exception {
    try (Context context = context()) {
      Path directory = Files.createTempDirectory("hara-core-language-deps-");
      Path child = directory.resolve("child.hal");
      Path parent = directory.resolve("parent.hal");
      try {
        String childPath = child.toString().replace("\\", "\\\\").replace("\"", "\\\"");
        String parentPath = parent.toString().replace("\\", "\\\\").replace("\"", "\\\"");
        Files.writeString(child, "(def child-value 7)");
        Files.writeString(parent, "(require \"" + childPath + "\") (def parent-value 8)");
        context.eval(HaraLanguage.ID, "(require \"" + parentPath + "\")");
        assertEquals(
            child.toAbsolutePath().normalize().toString(),
            context
                .eval(HaraLanguage.ID, "(nth (module-dependencies \"" + parentPath + "\") 0)")
                .asString());
      } finally {
        Files.deleteIfExists(parent);
        Files.deleteIfExists(child);
        Files.deleteIfExists(directory);
      }
    }
  }

  @Test
  public void evaluatesMultipleTopLevelFormsAndNamespaces() {
    try (Context context = context()) {
      assertEquals(3, context.eval(HaraLanguage.ID, "(def x 1) (+ x 2)").asLong());
      assertEquals(7, context.eval(HaraLanguage.ID, "(ns alpha) (def x 7) x").asLong());
      assertEquals(7, context.eval(HaraLanguage.ID, "alpha/x").asLong());

      context.eval(HaraLanguage.ID, "(ns beta)");
      PolyglotException missing =
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "x"));
      assertTrue(missing.getMessage().contains("Unbound symbol: x"));
      context.eval(HaraLanguage.ID, "(ns user)");
      assertEquals(1, context.eval(HaraLanguage.ID, "user/x").asLong());
    }
  }

  @Test
  public void resolvesQualifiedSymbolsThroughContextLocalAliases() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns target) (def answer 42) (ns user) (alias t target) t/answer")
              .asLong());
      PolyglotException missing =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(alias x absent)"));
      assertTrue(missing.getMessage().contains("missing namespace"));
    }
  }

  @Test
  public void refersNamespaceValuesIntoCurrentNamespace() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns source) (def answer 42) (ns user) (refer \"source\") answer")
              .asLong());
      PolyglotException missing =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(refer \"absent\")"));
      assertTrue(missing.getMessage().contains("missing namespace"));
    }
  }

  @Test
  public void refersLiveVarIdentityAcrossNamespaces() {
    try (Context context = context()) {
      assertEquals(
          2,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns source) (def answer 1) (ns user) (refer \"source\") "
                      + "(in-ns 'source) (set! answer 2) (in-ns 'user) answer")
              .asLong());
    }
  }

  @Test
  public void altersVarRootThroughAnHaraFunction() {
    try (Context context = context()) {
      assertEquals(
          41,
          context
              .eval(
                  HaraLanguage.ID,
                  "(def answer 1) (defn add [x y] (+ x y)) "
                      + "(alter-var-root (var answer) add 40) answer")
              .asLong());
      assertEquals(41, context.eval(HaraLanguage.ID, "answer").asLong());
    }
  }

  @Test
  public void altersVarRootToABuiltinKeepsItCallable() {
    try (Context context = context()) {
      assertEquals(
          "ab",
          context
              .eval(
                  HaraLanguage.ID,
                  "(def c count) (alter-var-root (var c) (fn [_] str)) (c \"a\" \"b\")")
              .asString());
      assertEquals(
          2,
          context.eval(HaraLanguage.ID, "(def y 1) (alter-var-root (var y) inc) y").asLong());
    }
  }

  @Test
  public void assocCoercesLongVectorIndices() {
    try (Context context = context()) {
      assertEquals(
          "[:x 2 3]", context.eval(HaraLanguage.ID, "(str (assoc [1 2 3] 0 :x))").asString());
      assertEquals(
          "[1 2 3 :x]", context.eval(HaraLanguage.ID, "(str (assoc [1 2 3] 3 :x))").asString());
      PolyglotException error =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(assoc [1 2 3] :k :x)"));
      assertTrue(error.getMessage().contains("assoc index must be a number"));
    }
  }

  @Test
  public void lazyIteratorsPrintWithAStableDisplayString() {
    try (Context context = context()) {
      assertEquals(
          "#<lazy-iterator>",
          context
              .eval(HaraLanguage.ID, "(str (Iter/iter-map (fn [x] (+ x 1)) [1 2 3]))")
              .asString());
    }
  }

  @Test
  public void appliesFunctionsWithAFinalSequentialArgument() {
    try (Context context = context()) {
      assertEquals(
          6,
          context
              .eval(HaraLanguage.ID, "(defn sum3 [a b c] (+ a b c)) (apply sum3 1 [2 3])")
              .asLong());
      assertEquals(
          1,
          context
              .eval(HaraLanguage.ID, "(defn first-rest [x & xs] x) (apply first-rest [1 2 3 4])")
              .asLong());
      assertEquals(
          "Ada",
          context
              .eval(
                  HaraLanguage.ID,
                  "(defstruct Person [name]) (:name (apply Person [\"Ada\"]))")
              .asString());
    }
  }

  @Test
  public void supportsInNsAndUseAsOrdinaryRuntimeForms() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(in-ns 'source) (def answer 42) (in-ns 'user) (use 'source) answer")
              .asLong());
      PolyglotException invalid =
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(in-ns 1)"));
      assertTrue(invalid.getMessage().contains("unqualified namespace symbol"));
    }
  }

  @Test
  public void isolatesDefinitionsBetweenContexts() {
    try (Context first = context();
        Context second = context()) {
      first.eval(HaraLanguage.ID, "(def only-here 1)");
      assertEquals(1, first.eval(HaraLanguage.ID, "only-here").asLong());
      PolyglotException missing =
          assertThrows(PolyglotException.class, () -> second.eval(HaraLanguage.ID, "only-here"));
      assertTrue(missing.getMessage().contains("Unbound symbol: only-here"));
    }
  }

  @Test
  public void returnsPolyglotExecutableFunctions() {
    try (Context context = context()) {
      Value increment = context.eval(HaraLanguage.ID, "(fn [x] (+ x 1))");
      assertTrue(increment.canExecute());
      assertEquals(42, increment.execute(41).asLong());
      assertEquals(42, context.eval(HaraLanguage.ID, "((fn [x] (+ x 1)) 41)").asLong());
    }
  }

  @Test
  public void destructuresSequentialFunctionArguments() {
    try (Context context = context()) {
      assertEquals(
          42,
          context.eval(HaraLanguage.ID, "((fn [[left right]] (+ left right)) [19 23])").asLong());
    }
  }

  @Test
  public void supportsVariadicFunctionRestArguments() {
    try (Context context = context()) {
      assertEquals(
          42, context.eval(HaraLanguage.ID, "((fn [value & more] value) 42 1 2)").asLong());
      PolyglotException error =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "((fn [value & more] value))"));
      assertTrue(error.getMessage().contains("at least 1 arguments"));
    }
  }

  @Test
  public void dispatchesMultiArityFnAndDefnClauses() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(defn choose "
                      + "([value] value) "
                      + "([left right] (+ left right))) "
                      + "(choose 19 23)")
              .asLong());
      assertEquals(41, context.eval(HaraLanguage.ID, "(choose 41)").asLong());
    }
  }

  @Test
  public void destructuresMapFunctionArguments() {
    try (Context context = context()) {
      assertEquals(
          42, context.eval(HaraLanguage.ID, "((fn [{:keys [age]}] (+ age 1)) {:age 41})").asLong());
      assertEquals(
          42,
          context
              .eval(HaraLanguage.ID, "((fn [{:keys [age] :or {age 41}}] (+ age 1)) {})")
              .asLong());
      assertEquals(
          "[42 {:answer 42}]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [{answer :answer :as whole} {:answer 42}] [answer whole])")
              .toString());
    }
  }

  @Test
  public void destructuresConsSequencesAndExposesPortableCompatibilityPrimitives() {
    try (Context context = context()) {
      assertEquals(
          "[1 2 [3]]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [[first second & more] (cons 1 (cons 2 (cons 3 nil)))] "
                      + "[first second more])")
              .toString());
      assertEquals(
          "[true false 42 nil true nil nil nil]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(cons? (cons 1 nil)) (list? (cons 1 nil)) "
                      + "(std.native.Num/parse-long \"42\") "
                      + "(std.native.Num/parse-long \"4x\") "
                      + "(= (std.native.Num/parse-double \"3.5\") "
                      + "   (std.native.Num/double 3.5)) "
                      + "(std.native.Num/parse-double \"3x\") "
                      + "(std.native.String/split \"\" \",\") "
                      + "(std.native.RegExp/split (std.native.RegExp/compile \",\") \"\")]" )
              .toString());
    }
  }

  @Test
  public void destructuresLetAndLoopBindingsIncludingNestedRest() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(HaraLanguage.ID, "(let [[left & rest] [19 23]] (if rest (+ left 23) 0))")
              .asLong());
      assertEquals(
          42, context.eval(HaraLanguage.ID, "(let [{:keys [age]} {:age 41}] (+ age 1))").asLong());
      assertEquals(
          42,
          context
              .eval(HaraLanguage.ID, "(loop [[left & rest] [19 23]] (if rest (+ left 23) 0))")
              .asLong());
    }
  }

  @Test
  public void destructuringTreatsMissingValuesAndNilSourcesAsNil() {
    try (Context context = context()) {
      assertEquals(
          41,
          context.eval(HaraLanguage.ID, "(let [[a b] [1]] (+ (if a 1 0) (if b 40 40)))").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(let [{:keys [a]} nil] (if a 1 2))").asLong());
      assertEquals(
          2, context.eval(HaraLanguage.ID, "(let [[a & rest] nil] (if rest 1 2))").asLong());
    }
  }

  @Test
  public void definesOrdinaryFunctionsWithOptionalDocumentationAndAttributes() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(defn increment \"increments\" {:private true} [x] (+ x 1)) " + "(increment 41)")
              .asLong());
    }
  }

  @Test
  public void userFunctionsExposeDocstringsAndArglists() {
    try (Context context = context()) {
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(do (defn add \"adds values\" [left right] (+ left right)) "
                  + "(let [m (meta #'add)] [(get m :doc) (get m :arglists)]))");
      assertEquals("adds values", result.getArrayElement(0).asString());
      assertEquals(2, result.getArrayElement(1).getArrayElement(0).getArraySize());
    }
  }

  @Test
  public void resolvesRecursiveDefnCallsThroughTheCurrentNamespace() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(defn recurse-once [value] "
                      + "  (if value (recurse-once nil) 42)) "
                      + "(recurse-once true)")
              .asLong());
    }
  }

  @Test
  public void capturesLexicalBindingsInReturnedFunctions() {
    try (Context context = context()) {
      Value adder = context.eval(HaraLanguage.ID, "(let [x 41] (fn [y] (+ x y)))");
      assertTrue(adder.canExecute());
      assertEquals(42, adder.execute(1).asLong());
      assertEquals(
          42, context.eval(HaraLanguage.ID, "(((fn [x] (fn [y] (+ x y))) 40) 2)").asLong());
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [x 40]"
                      + " ((fn [y] ((fn [z] (+ x y z)) 1)) 1))")
              .asLong());
    }
  }

  @Test
  public void capturesTheCorrectShadowedBinding() {
    try (Context context = context()) {
      assertEquals(
          10,
          context
              .eval(HaraLanguage.ID, "(let [x 10] (let [f (fn [] x)] (let [x 20] (f))))")
              .asLong());
    }
  }

  @Test
  public void definesImmutableHostIndependentStructs() {
    try (Context context = context()) {
      Value person =
          context.eval(HaraLanguage.ID, "(defstruct Person [name age]) (Person \"Ada\" 36)");
      assertTrue(person.hasMembers());
      assertEquals("Ada", person.getMember("name").asString());
      assertEquals(36, person.getMember("age").asLong());
      assertTrue(person.toString().contains("Person"));
      assertEquals(
          "Ada", context.eval(HaraLanguage.ID, "(:name (Person \"Ada\" 36))").asString());
      assertTrue(context.eval(HaraLanguage.ID, "Person").canExecute());
    }
  }

  @Test
  public void structsCarryLanguageNativeMetadata() {
    try (Context context = context()) {
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(defstruct Person [name]) "
                  + "(ILookup/lookup "
                  + "  (IObjType/meta "
                  + "    (IObjType/with-meta (Person \"Ada\") {:doc \"person\"})) "
                  + "  :doc)");
      assertEquals("person", result.asString());
    }
  }

  @Test
  public void namedStructsExposeOneCanonicalSchemaForm() {
    try (Context context = context()) {
      assertEquals(
          "[[:struct (var user/Person) [:name :str] [:age {:optional true} :int]] "
              + "[:struct {:mutable? true} (var user/Cursor) "
              + "[:position :int] [:limit {:optional true} :int]]]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (defstruct Person [[name :str] [age {:optional true} :int]]) "
                      + "    (defmutable Cursor [[position :int] "
                      + "                         [limit {:optional true} :int]]) "
                      + "    [(std.native.Schema/form (std.native.Schema/of (var Person))) "
                      + "     (std.native.Schema/form (std.native.Schema/of (var Cursor)))])")
              .toString());
    }
  }

  @Test
  public void extendsStructsWithLanguageProtocolsIncludingIFn() {
    try (Context context = context()) {
      assertEquals(
          43,
          context
              .eval(
                  HaraLanguage.ID,
                  "(defstruct Counter [base]) "
                      + "(defprotocol CounterOps (value [self]) (add [self amount])) "
                      + "(extend-type Counter CounterOps "
                      + "  (value [self] (:base self)) "
                      + "  (add [self amount] (+ (:base self) amount))) "
                      + "(add (Counter 41) 2)")
              .asLong());

      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(defstruct Incrementer [base]) "
                      + "(extend-type Incrementer IFn "
                      + "  (invoke [self value] (+ (:base self) value))) "
                      + "((Incrementer 1) 41)")
              .asLong());
    }
  }

  @Test
  public void protocolMethodsAllowPredicatesAndBangNames() {
    try (Context context = context()) {
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(defprotocol PredicateProtocol (ready? [self]))")
              .toString()
              .contains("user/PredicateProtocol"));
      assertTrue(
          context
              .eval(HaraLanguage.ID, "user/ready?")
              .toString()
              .contains("user/ready?"));
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(defprotocol MutatingProtocol (mutate! [self]))")
              .toString()
              .contains("user/MutatingProtocol"));
      assertTrue(
          context
              .eval(HaraLanguage.ID, "user/mutate!")
              .toString()
              .contains("user/mutate!"));
    }
  }

  @Test
  public void guestProtocolMethodsAreDirectReloadableAndCollisionSafe() {
    try (Context context = context()) {
      assertEquals(
          "[41 42]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(defstruct Box [value]) "
                      + "(defprotocol BoxOps (read [self])) "
                      + "(extend-type Box BoxOps (read [self] (:value self))) "
                      + "[(read (Box 41)) (user/read (Box 42))]")
              .toString());

      assertTrue(
          context
              .eval(HaraLanguage.ID, "(defprotocol BoxOps (read [self]))")
              .toString()
              .contains("user/BoxOps"));

      PolyglotException collision =
          assertThrows(
              PolyglotException.class,
              () ->
                  context.eval(
                      HaraLanguage.ID,
                      "(def ordinary 1) (defprotocol Broken (fresh [self]) (ordinary [self]))"));
      assertTrue(collision.getMessage().contains("Protocol method Var already exists"));
      assertEquals("1", context.eval(HaraLanguage.ID, "ordinary").toString());
      assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "fresh"));
      assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "Broken"));

      assertThrows(
          PolyglotException.class,
          () -> context.eval(HaraLanguage.ID, "(protocol-call BoxOps read (Box 1))"));
      assertThrows(
          PolyglotException.class,
          () -> context.eval(HaraLanguage.ID, "(BoxOps/read (Box 1))"));
    }
  }

  @Test
  @org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
  public void exposesTheSharedProtocolInventoryFromFoundation() throws Exception {
    String contract =
        Files.readString(
            specsRegistry()
                .resolve("01-lang/001-language/draft/conformance/protocols.edn"));
    Matcher names = Pattern.compile(":name\\s+(I[A-Za-z]+)").matcher(contract);
    Set<String> protocols = new LinkedHashSet<>();
    while (names.find()) {
      protocols.add(names.group(1));
    }
    assertTrue(contract, contract.contains(":protocol-count 61"));
    assertTrue(contract, contract.contains(":capability-specific-protocol-count 15"));
    assertEquals(76, protocols.size());
    Set<String> unavailableProtocols =
        Set.of(
            "IHasRuntime",
            "IRanged",
            "IValidate",
            "IComponentOptions",
            "IComponentProps",
            "IComponentQuery",
            "IComponentTrack");
    try (Context context = context()) {
      for (String protocol : protocols) {
        String protocolNamespace = "std.protocol." + protocol.toLowerCase(java.util.Locale.ROOT);
        assertTrue(
            protocol,
            context
                .eval(HaraLanguage.ID, protocolNamespace + "." + protocol + "/" + protocol)
                .toString()
                .contains(protocolNamespace + "." + protocol));
        assertTrue(
            protocol,
            context
                .eval(
                    HaraLanguage.ID,
                    "(= std.foundation/"
                        + protocol
                        + " "
                        + protocolNamespace
                        + "."
                        + protocol
                        + "/"
                        + protocol
                        + ")")
                .asBoolean());
      }
      assertEquals(
          3L,
          context
              .eval(HaraLanguage.ID, "(std.protocol.icount.ICount/count [1 2 3])")
              .asLong());
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(std.protocol.icas.ICas/cas (atom 1) 1 2)")
              .asBoolean());
      assertEquals(
          6L,
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.protocol.ireduce.IReduce/reduce [1 2 3] + 0)")
              .asLong());
      assertEquals(
          ":fulfilled",
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.protocol.ipromise.IPromise/state (std.foundation.promise/from 7))")
              .toString());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= [:a 1] (std.protocol.ifind.IFind/find {:a 1} :a))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(satisfies? std.protocol.ipeekfirst.IPeekFirst [1])")
              .asBoolean());
      PolyglotException legacyProtocolMethod =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "std.protocol.ipeekfirst/peek-first"));
      assertTrue(legacyProtocolMethod.getMessage().contains("Unbound symbol"));
      PolyglotException obsoleteMethod =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "std.foundation/ICount/count"));
      assertTrue(obsoleteMethod.getMessage().contains("Unbound symbol"));
      for (String unavailableProtocol : unavailableProtocols) {
        String hiddenNamespace =
            "std.protocol." + unavailableProtocol.toLowerCase(java.util.Locale.ROOT);
        PolyglotException hiddenCanonical =
            assertThrows(
                PolyglotException.class,
                () ->
                    context.eval(
                        HaraLanguage.ID,
                        hiddenNamespace + "." + unavailableProtocol + "/" + unavailableProtocol));
        assertTrue(hiddenCanonical.getMessage().contains("Unbound symbol"));
        PolyglotException hiddenFoundation =
            assertThrows(
                PolyglotException.class,
                () ->
                    context.eval(
                        HaraLanguage.ID,
                        "std.foundation/" + unavailableProtocol));
        assertTrue(hiddenFoundation.getMessage().contains("Unbound symbol"));
      }
    }
  }

  @Test
  @org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
  public void sharedFoundationProtocolConformanceFixtureRuns() throws Exception {
    String protocols =
        Files.readString(
            specsRegistry()
                .resolve("01-lang/001-language/draft/conformance/protocols.edn"));
    String source =
        Files.readString(
            specsRegistry()
                .resolve(
                    "01-lang/001-language/draft/conformance/fixtures/protocol_surface.hal"));
    Matcher calls =
        Pattern.compile("\\(std\\.protocol\\.[a-z]+\\.I[A-Za-z]+/[a-z?\\-]+\\s+fixture")
            .matcher(source);
    int callCount = 0;
    while (calls.find()) callCount++;
    assertEquals(109, callCount);
    assertTrue(!source.contains("protocol-call"));
    assertTrue(
        "protocol types must resolve unqualified in guest source",
        !Pattern.compile("std\\.protocol\\.[^\\s/]+/I[A-Z]").matcher(source).find());

    try (Context context = context()) {
      String result = context.eval(HaraLanguage.ID, source).toString();
      assertTrue(result, !result.contains(":pass false"));
      assertEquals(57, result.split(":pass true", -1).length - 1);
    }
  }

  @Test
  @org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
  public void sharedFoundationProtocolFunctionalityFixtureRuns() throws Exception {
    String catalog =
        Files.readString(
            specsRegistry()
                .resolve(
                    "01-lang/001-language/draft/conformance/protocol-method-cases.edn"));
    String protocols =
        Files.readString(
            specsRegistry()
                .resolve("01-lang/001-language/draft/conformance/protocols.edn"));
    String source =
        Files.readString(
            specsRegistry()
                .resolve(
                    "01-lang/001-language/draft/conformance/fixtures/protocol_behavioral.hal"));
    assertTrue(
        "protocol types must resolve unqualified in guest source",
        !Pattern.compile("std\\.protocol\\.[^\\s/]+/I[A-Z]").matcher(source).find());
    Set<String> specifiedMethods = new LinkedHashSet<>();
    Matcher protocolEntries =
        Pattern.compile(
                "\\{:name\\s+(I[A-Za-z]+).*?:methods\\s+\\{([^}]*)\\}", Pattern.DOTALL)
            .matcher(protocols);
    while (protocolEntries.find()) {
      String protocol = protocolEntries.group(1);
      Matcher methods = Pattern.compile("([a-z][a-z?\\-]*)\\s+-?\\d+").matcher(protocolEntries.group(2));
      while (methods.find()) specifiedMethods.add(protocol + "/" + methods.group(1));
    }
    Set<String> catalogMethods = new LinkedHashSet<>();
    Matcher catalogEntries =
        Pattern.compile("\\{:protocol\\s+(I[A-Za-z]+)\\s+:method\\s+([^\\s]+)")
            .matcher(catalog);
    while (catalogEntries.find()) {
      catalogMethods.add(catalogEntries.group(1) + "/" + catalogEntries.group(2));
    }
    assertEquals(
        "Behavioral protocol cases must exactly close the authoritative method surface",
        specifiedMethods,
        catalogMethods);
    assertEquals(129, catalog.split("\\{:protocol ", -1).length - 1);
    assertEquals(6, protocols.split(" -1", -1).length - 1);
    assertEquals(6, catalog.split(":case :declared-variadic", -1).length - 1);
    int expectedFailureCount =
        catalog.split(":case :unsupported-receiver", -1).length - 1;
    Matcher methodVars =
        Pattern.compile(
                "(?m)^\\s*\\[?\\(protocol-case\\s+:[^\\s]+\\s+:[^\\s]+\\s+"
                    + "(std\\.protocol\\.[a-z]+\\.I[A-Za-z]+/[a-z?\\-]+)")
            .matcher(source);

    try (Context context = context()) {
      String result = context.eval(HaraLanguage.ID, source).toString();
      assertTrue(result, !result.contains(":pass false"));
      assertEquals(109, result.split(":pass true", -1).length - 1);
      String capabilityResult =
          context.eval(HaraLanguage.ID, "(capability-protocol-results)").toString();
      assertTrue(capabilityResult, !capabilityResult.contains(":pass false"));
      assertEquals(20, capabilityResult.split(":pass true", -1).length - 1);
      String receiverMatrix =
          context.eval(HaraLanguage.ID, "(protocol-receiver-matrix-results)").toString();
      assertTrue(receiverMatrix, !receiverMatrix.contains(":pass false"));
      assertEquals(10, receiverMatrix.split(":pass true", -1).length - 1);
      String crossCutting =
          context.eval(HaraLanguage.ID, "(protocol-cross-cutting-results)").toString();
      assertTrue(crossCutting, !crossCutting.contains(":pass false"));
      assertEquals(6, crossCutting.split(":pass true", -1).length - 1);
      String capabilityReceivers =
          context
              .eval(HaraLanguage.ID, "(protocol-capability-receiver-results)")
              .toString();
      assertTrue(capabilityReceivers, !capabilityReceivers.contains(":pass false"));
      assertEquals(8, capabilityReceivers.split(":pass true", -1).length - 1);
      String nativeValues =
          context.eval(HaraLanguage.ID, "(protocol-native-value-results)").toString();
      assertTrue(nativeValues, !nativeValues.contains(":pass false"));
      assertEquals(15, nativeValues.split(":pass true", -1).length - 1);
      String predicates =
          context.eval(HaraLanguage.ID, "(protocol-predicate-results)").toString();
      assertTrue(predicates, !predicates.contains(":pass false"));
      assertEquals(7, predicates.split(":pass true", -1).length - 1);

      int methodCount = 0;
      while (methodVars.find()) {
        methodCount++;
        String methodVar = methodVars.group(1);
        PolyglotException error =
            assertThrows(
                methodVar,
                PolyglotException.class,
                () -> context.eval(HaraLanguage.ID, "(" + methodVar + ")"));
        assertTrue(
            methodVar + " returned an uncategorized arity error: " + error.getMessage(),
            error.getMessage().contains("protocol/arity"));
      }
      assertEquals(129, methodCount);

      int failureCount = 0;
      for (String line : source.split("\\R")) {
        int quote = line.indexOf("'(std.protocol.");
        if (quote < 0) continue;
        failureCount++;
        String quoted = line.substring(quote + 1);
        int depth = 0;
        int end = -1;
        for (int index = 0; index < quoted.length(); index++) {
          char character = quoted.charAt(index);
          if (character == '(') depth++;
          if (character == ')' && --depth == 0) {
            end = index + 1;
            break;
          }
        }
        assertTrue("unbalanced failure form: " + line, end > 0);
        String failureForm = quoted.substring(0, end);
        PolyglotException error =
            assertThrows(
                failureForm,
                PolyglotException.class,
                () -> context.eval(HaraLanguage.ID, failureForm));
        assertTrue(
            failureForm + " returned an uncategorized dispatch error: " + error.getMessage(),
            error.getMessage().contains("protocol/unsupported-receiver"));
      }
      assertEquals(expectedFailureCount, failureCount);
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (std.protocol.icount.ICount/count) false "
                      + "(catch Throwable error true))")
              .asBoolean());
    }
  }

  @Test
  @org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
  public void sharedFoundationProtocolCorpusRecordsJvmInterpreterCapabilityExclusion()
      throws Exception {
    String protocols =
        Files.readString(
            specsRegistry()
                .resolve("01-lang/001-language/draft/conformance/protocols.edn"));
    String corpus =
        Files.readString(
            specsRegistry()
                .resolve(
                    "01-lang/001-language/draft/conformance/fixtures/protocol_behavioral.hal"));
    assertTrue(corpus.contains("protocol-cross-cutting-results"));
    assertTrue(corpus.contains("capability-protocol-results"));
    assertTrue(corpus.contains("protocol-capability-receiver-results"));
    assertTrue(protocols.contains(":jvm-interpreter"));
    assertTrue(
        protocols.contains(
            ":portable 109 :capability-specific 20 :passed 0 :failed 0 :skipped 129"));
    assertTrue(
        protocols.contains(":reason :jvm-interpreter-does-not-expose-std-protocol-guest-vars"));
    assertEquals(4, protocols.split(":passed 129 :failed 0 :skipped 0", -1).length - 1);

    RT.Instance<Object> interpreter = new RT.Instance<>(null, "protocol-conformance-profile");
    assertThrows(
        Throwable.class,
        () ->
            interpreter.eval(
                interpreter.readString("std.protocol.icount/ICount")));
  }

  @Test
  public void cachesProtocolDispatchByReceiverShapeAndInvalidatesExtensions() {
    try (Context context = context()) {
      assertEquals(
          "Ada",
          context
              .eval(
                  HaraLanguage.ID,
                  "(defprotocol Describable (describe [self])) "
                      + "(defstruct Person [name]) "
                      + "(defstruct NumberValue [value]) "
                      + "(extend-type Person Describable "
                      + "  (describe [self] (:name self))) "
                      + "(extend-type NumberValue Describable "
                      + "  (describe [self] (:value self))) "
                      + "(def describe-value "
                      + "  (fn [value] (describe value))) "
                      + "(describe-value (Person \"Ada\"))")
              .asString());
      assertEquals(42, context.eval(HaraLanguage.ID, "(describe-value (NumberValue 42))").asLong());

      assertEquals(
          2,
          context
              .eval(
                  HaraLanguage.ID,
                  "(extend-type Person Describable (describe [self] 2)) "
                      + "(describe-value (Person \"Ada\"))")
              .asLong());
    }
  }

  @Test
  public void extendsStructsWithCoreAdapterProtocols() {
    try (Context context = context()) {
      assertEquals(
          41,
          context
              .eval(
                  HaraLanguage.ID,
                  "(defstruct Box [size]) "
                      + "(extend-type Box ICount "
                      + "  (count [self] (:size self))) "
                      + "(ICount/count (Box 41))")
              .asLong());
    }
  }

  @Test
  public void doesNotExposeJvmHostInteropForms() {
    try (Context context = context()) {
      for (String form :
          new String[] {
            "(host-symbol \"java.lang.String\")",
            "(host-get nil \"value\")",
            "(host-call nil \"run\")"
          }) {
        PolyglotException error =
            assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, form));
        assertTrue(error.getMessage().contains("Unbound symbol"));
      }
    }
  }

  @Test
  public void expandsMacrosBeforeTruffleAnalysis() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(defmacro unless [test body] `(if ~test nil ~body))");
      assertEquals(3, context.eval(HaraLanguage.ID, "(unless false (+ 1 2))").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(unless true (+ 1 2))").isNull());
    }
  }

  @Test
  public void expandsMacrosDefinedEarlierInTheSameSource() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(defmacro when-not [test body] `(if ~test nil ~body)) "
                      + "(when-not false (+ 40 2))")
              .asLong());
    }
  }

  @Test
  public void supportsUnquoteSplicingInSyntaxQuotedForms() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(defmacro do-all [forms] `(do ~@forms))");
      assertEquals(3, context.eval(HaraLanguage.ID, "(do-all (1 2 3))").asLong());
    }
  }

  @Test
  public void agreesWithTheExistingInterpreterForTheSupportedSlice() {
    RT.Instance<Object> interpreter = new RT.Instance<>(null, "truffle-differential-test");
    String[] expressions = {
      "(+ 19 23)",
      "(if false 1 2)",
      "(do 1 2 3)",
      "(let [x 2 y 3] (+ x y))",
      "((fn [x] (+ x 1)) 41)"
    };

    try (Context context = context()) {
      for (String expression : expressions) {
        Object interpreted = interpreter.eval(interpreter.readString(expression));
        assertTrue("interpreter returned nil for " + expression, interpreted instanceof Number);
        Number expected = (Number) interpreted;
        assertEquals(expected.longValue(), context.eval(HaraLanguage.ID, expression).asLong());
      }
    }
  }

  @Test
  public void reportsLanguageErrorsAtThePolyglotBoundary() {
    try (Context context = context()) {
      PolyglotException unbound =
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "missing"));
      assertTrue(unbound.getMessage().contains("Unbound symbol: missing"));
      assertTrue(unbound.getSourceLocation().isAvailable());
      assertEquals(1, unbound.getSourceLocation().getStartLine());
      assertTrue(unbound.getPolyglotStackTrace().iterator().hasNext());

      PolyglotException arity =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "((fn [x] x) 1 2)"));
      assertTrue(arity.getMessage().contains("Expected 1 arguments, received 2"));

      assertEquals(1, context.eval(HaraLanguage.ID, "(let [x 1] ((fn [] x)))").asLong());
    }
  }

  @Test
  public void supportsExplicitJvmFlavorImportsConstructionAndDotChains() {
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .allowHostAccess(HostAccess.ALL)
            .allowHostClassLookup(name -> true)
            .build()) {
      context.eval(
          HaraLanguage.ID,
          "(ns jvm-test (:flavor :jvm [java.lang String RuntimeException] [java.awt Point]))");
      assertEquals(
          "HELLO",
          context.eval(HaraLanguage.ID, "(. (new String \"hello\") (toUpperCase))").asString());
      assertEquals("42", context.eval(HaraLanguage.ID, "(String/valueOf 42)").asString());
      assertEquals(
          9,
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [p (new Point 3 4)] (hara.native.jvm/set! p \"x\" 9) (. p x))")
              .asLong());
      assertEquals(
          "boom",
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (throw (new RuntimeException \"boom\")) (catch RuntimeException e (. e (getMessage))))")
              .asString());
      assertEquals(
          "3", context.eval(HaraLanguage.ID, "(. (new Point 3 4) x (toString))").asString());
    }
  }

  @Test
  public void defaultsNativeImportsToJvm() {
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .allowHostAccess(HostAccess.ALL)
            .allowHostClassLookup(name -> true)
            .build()) {
      context.eval(HaraLanguage.ID, "(ns jvm-default (:flavor :jvm [java.lang String]))");
      assertEquals("42", context.eval(HaraLanguage.ID, "(String/valueOf 42)").asString());
    }
  }

  @Test
  public void selectingJvmFlavorDoesNotGrantReflectionAuthority() {
    try (Context context = context()) {
      PolyglotException error =
          assertThrows(
              PolyglotException.class,
              () ->
                  context.eval(
                      HaraLanguage.ID,
                      "(ns denied (:flavor :jvm [java.lang String RuntimeException]))"));
      assertTrue(error.getMessage().contains("reflection capability is not granted"));
    }
  }

  @Test
  public void specializesCollectionGetNthAssocOnBuiltins() {
    try (Context context = context()) {
      assertEquals(1, context.eval(HaraLanguage.ID, "(get {:a 1} :a)").asLong());
      assertEquals(10, context.eval(HaraLanguage.ID, "(get [10 20] 0)").asLong());
      assertEquals(30, context.eval(HaraLanguage.ID, "(nth [10 20 30] 2)").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(get (assoc {:a 1} :b 2) :b)").asLong());
      assertEquals(5, context.eval(HaraLanguage.ID, "(get (assoc {:a 1} :a 5) :a)").asLong());
    }
  }

  @Test
  public void collectionGetHandlesMissingKeysNilAndDefaults() {
    try (Context context = context()) {
      assertTrue(context.eval(HaraLanguage.ID, "(get {:a 1} :b)").isNull());
      assertTrue(context.eval(HaraLanguage.ID, "(= :d (get {:a 1} :b :d))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(get nil :k)").isNull());
      assertTrue(context.eval(HaraLanguage.ID, "(= :d (get nil :k :d))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(= 1 (get {:a 1} :a :d))").asBoolean());
    }
  }

  @Test
  public void collectionGetHandlesSetReceivers() {
    try (Context context = context()) {
      assertEquals(2, context.eval(HaraLanguage.ID, "(get #{1 2} 2)").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(get #{1 2} 9)").isNull());
      assertTrue(context.eval(HaraLanguage.ID, "(= :missing (get #{1 2} 9 :missing))").asBoolean());
      assertEquals(2, context.eval(HaraLanguage.ID, "(get (hash-set 1 2) 2)").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(ILookup/lookup #{1 2} 2)").asLong());
      // Sets answer get but, matching the native runtime, do not satisfy ILookup.
      assertTrue(context.eval(HaraLanguage.ID, "(not (lookupable? #{1 2}))").asBoolean());
    }
  }

  @Test
  public void hasHandlesAssociativeCollectionKeys() {
    try (Context context = context()) {
      assertTrue(context.eval(HaraLanguage.ID, "(has? {:a 1} :a)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(has? {:a nil} :a)").asBoolean());
      assertTrue(!context.eval(HaraLanguage.ID, "(has? {:a 1} :b)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(has? #{:a} :a)").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(has? [10 20] 1)").asBoolean());
      assertTrue(!context.eval(HaraLanguage.ID, "(has? [10 20] 20)").asBoolean());
    }
  }

  @Test
  public void collectionNthPreservesBoundsAndArityFailures() {
    try (Context context = context()) {
      PolyglotException bounds =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(nth [1 2] 5)"));
      assertTrue(bounds.getMessage().contains("nth index out of bounds"));
      PolyglotException arity =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(nth [1 2] 5 :d)"));
      assertTrue(arity.getMessage().contains("protocol/arity"));
      PolyglotException getArity =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(get {:a 1})"));
      assertTrue(getArity.getMessage().contains("ILookup/lookup expects one or two arguments"));
    }
  }

  @Test
  public void assocDoesNotMutateTheOriginalCollection() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(def original {:a 1}) (def updated (assoc original :b 2))");
      assertTrue(
          context.eval(HaraLanguage.ID, "(= :none (get original :b :none))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(= 2 (get updated :b))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(= 1 (get updated :a))").asBoolean());
    }
  }

  @Test
  public void assocMetadataAgreesWithGenericProtocolDispatch() {
    try (Context context = context()) {
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= {:tag :x} (meta (with-meta {:a 1} {:tag :x})))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= (meta (assoc (with-meta {:a 1} {:tag :x}) :b 2))"
                      + " (meta (apply assoc [(with-meta {:a 1} {:tag :x}) :b 2])))")
              .asBoolean());
    }
  }

  @Test
  public void collectionOpsUseCustomProtocolImplementations() {
    try (Context context = context()) {
      context.eval(
          HaraLanguage.ID,
          "(defstruct Box [m]) (extend-type Box ILookup (lookup [self k] :custom))");
      assertTrue(context.eval(HaraLanguage.ID, "(= :custom (get (Box nil) :k))").asBoolean());
    }
  }

  @Test
  public void collectionOpsSeeProtocolExtensionsImmediately() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(defstruct Late [m])");
      PolyglotException unsupported =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(nth (Late nil) 0)"));
      assertTrue(unsupported.getMessage().contains("unsupported-receiver"));
      context.eval(HaraLanguage.ID, "(extend-type Late INth (nth [self i] :extended))");
      assertTrue(
          context.eval(HaraLanguage.ID, "(= :extended (nth (Late nil) 0))").asBoolean());
      context.eval(HaraLanguage.ID, "(extend-type Late INth (nth [self i] :replaced))");
      assertTrue(
          context.eval(HaraLanguage.ID, "(= :replaced (nth (Late nil) 0))").asBoolean());
    }
  }

  @Test
  public void collectionOpsHandleMixedReceiverShapesAtOneCallSite() {
    try (Context context = context()) {
      context.eval(
          HaraLanguage.ID,
          "(defstruct Box [m]) (extend-type Box ILookup (lookup [self k] (:m self))) "
              + "(defn pick [c] (get c :a))");
      assertEquals(1, context.eval(HaraLanguage.ID, "(pick {:a 1})").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(pick nil)").isNull());
      assertEquals(9, context.eval(HaraLanguage.ID, "(pick (Box 9))").asLong());
      assertEquals(2, context.eval(HaraLanguage.ID, "(pick {:a 2})").asLong());
    }
  }

  @Test
  public void collectionOpsPreserveArgumentEvaluationOrder() {
    try (Context context = context()) {
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (def a (atom [])) "
                      + "(get (do (swap! a conj :recv) {:a 1}) (do (swap! a conj :key) :a)) "
                      + "(= [:recv :key] @a))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (def b (atom [])) "
                      + "(assoc (do (swap! b conj 1) {:x 1}) "
                      + "       (do (swap! b conj 2) :y) "
                      + "       (do (swap! b conj 3) 2)) "
                      + "(= [1 2 3] @b))")
              .asBoolean());
    }
  }

  @Test
  public void collectionOpsFallBackToGenericInvocationAfterRedefinition() {
    try (Context context = context()) {
      context.eval(
          HaraLanguage.ID,
          "(alter-var-root (var get) (fn [old] (fn [m k] :redefined)))");
      assertTrue(context.eval(HaraLanguage.ID, "(= :redefined (get {:a 1} :a))").asBoolean());
      assertTrue(context.eval(HaraLanguage.ID, "(= :redefined (get {:a 1} :b))").asBoolean());
    }
  }

  @Test
  public void collectionOpsRespectLexicalShadowing() {
    try (Context context = context()) {
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= :shadowed (let [get (fn [m k] :shadowed)] (get {:a 1} :a)))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= :sh ((fn [assoc] (assoc {:a 1} :b 2)) (fn [m k v] :sh)))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= :sh ((fn [nth] (nth [1 2] 0)) (fn [v i] :sh)))")
              .asBoolean());
    }
  }

  @Test
  public void invokesBuiltinsPassedAsValues() {
    try (Context context = context()) {
      assertEquals(
          "12", context.eval(HaraLanguage.ID, "((fn [f] (f 1 2)) str)").asString());
      assertEquals(2, context.eval(HaraLanguage.ID, "((fn [f] (f [1 2])) count)").asLong());
      assertEquals(
          "hara",
          context
              .eval(HaraLanguage.ID, "((fn [f g] (g (f \"ha\") (f \"ra\"))) str str)")
              .asString());
      assertEquals(
          "1",
          context.eval(HaraLanguage.ID, "(first (Iter/iter-map str [1 2 3]))").asString());
    }
  }

  @Test
  public void builtinFastPathPreservesArityErrors() {
    try (Context context = context()) {
      PolyglotException twoArgs =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(count 1 2)"));
      assertTrue(twoArgs.getMessage().contains("Wrong number of args (2)"));
      PolyglotException zeroArgs =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(count)"));
      assertTrue(zeroArgs.getMessage().contains("Wrong number of args (0)"));
    }
  }

  @Test
  public void builtinFastPathPreservesArgumentEvaluationOrder() {
    try (Context context = context()) {
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (def a (atom [])) "
                      + "((fn [f] (f (swap! a conj 1) (swap! a conj 2))) str) "
                      + "(= [1 2] @a))")
              .asBoolean());
    }
  }

  @Test
  public void builtinFastPathAppliesThroughGenericDispatchAfterRedefinition() {
    try (Context context = context()) {
      // Redefine get to a builtin: the CollectionOp generic fallback must invoke it
      // through the same builtin fast path as a plain Invoke node.
      context.eval(
          HaraLanguage.ID,
          "(ns fast-path (:config {:blank true}) "
              + "(:require [std.foundation :refer :all :exclude [get]])) (def get str)");
      assertEquals("ab", context.eval(HaraLanguage.ID, "(get \"a\" \"b\")").asString());
    }
  }

  @Test
  public void firstRestFastPathVectorsAndNil() {
    try (Context context = context()) {
      assertEquals(1, context.eval(HaraLanguage.ID, "(first [1 2 3])").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(first [])").isNull());
      assertTrue(context.eval(HaraLanguage.ID, "(first '())").isNull());
      assertTrue(context.eval(HaraLanguage.ID, "(first (vec []))").isNull());
      assertTrue(context.eval(HaraLanguage.ID, "(first nil)").isNull());
      assertEquals(2, context.eval(HaraLanguage.ID, "(first (rest [1 2 3]))").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(rest [1])").isNull());
      assertTrue(context.eval(HaraLanguage.ID, "(rest [])").isNull());
      assertTrue(context.eval(HaraLanguage.ID, "(rest nil)").isNull());
      assertEquals(3, context.eval(HaraLanguage.ID, "(first (rest (rest [1 2 3])))").asLong());
    }
  }

  @Test
  public void consUsesOneSequenceRepresentationForCompactAndTreeVectors() {
    try (Context context = context()) {
      assertEquals(
          "[:std.native.Cons (0 1 2) :std.native.Cons (0 1 2)]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [compact [1 2] tree (vec [1 2])] "
                      + "[(type (cons 0 compact)) (cons 0 compact) "
                      + " (type (cons 0 tree)) (cons 0 tree)])")
              .toString());
      assertEquals(
          ":std.native.List",
          context.eval(HaraLanguage.ID, "(type ((fn [& rest] rest) 1 2))").toString());
    }
  }

  @Test
  public void firstFastPathConsumesIterators() {
    try (Context context = context()) {
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= [1 2] (let [it (iter [1 2 3])] [(first it) (first it)]))")
              .asBoolean());
    }
  }

  @Test
  public void firstRestFastPathStringsAndMaps() {
    try (Context context = context()) {
      assertEquals(
          "a", context.eval(HaraLanguage.ID, "(str (first \"abc\"))").asString());
      assertEquals(
          "b", context.eval(HaraLanguage.ID, "(str (first (rest \"abc\")))").asString());
      assertTrue(
          context.eval(HaraLanguage.ID, "(= :a (first (first {:a 1})))").asBoolean());
    }
  }

  @Test
  public void firstRestHonorShadowingAndRedefinition() {
    try (Context context = context()) {
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(= :shadowed (let [first (fn [x] :shadowed)] (first [1 2 3])))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns sequence-shadow (:config {:blank true}) "
                      + "(:require [std.foundation :refer :all :exclude [first rest]])) "
                      + "(= :redefined (do (def first (fn [x] :redefined)) (first [1 2 3])))")
              .asBoolean());
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(= :r (do (def rest (fn [x] :r)) (rest [1 2 3])))")
              .asBoolean());
    }
  }

  @Test
  public void firstRestFastPathUnsupportedReceivers() {
    try (Context context = context()) {
      PolyglotException firstError =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(first 42)"));
      assertTrue(firstError.getMessage().contains("iter does not support value: 42"));
      PolyglotException restError =
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(rest 42)"));
      assertTrue(restError.getMessage().contains("iter does not support value: 42"));
    }
  }

  @Test
  public void firstRestArityErrors() {
    try (Context context = context()) {
      PolyglotException zeroArgs =
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(first)"));
      assertTrue(zeroArgs.getMessage().contains("Expected 1 arguments, received 0"));
      PolyglotException twoArgs =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(first 1 2)"));
      assertTrue(twoArgs.getMessage().contains("Expected 1 arguments, received 2"));
    }
  }

  @Test
  public void firstRestSurviveFoundationReload() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(require \"std/foundation.hal\" {:reload true})");
      assertEquals(7, context.eval(HaraLanguage.ID, "(first [7 8 9])").asLong());
      assertEquals(8, context.eval(HaraLanguage.ID, "(first (rest [7 8 9]))").asLong());
    }
  }

  @Test
  public void catchSelectorsMatchStructuredErrorCodes() {
    try (Context context = context()) {
      assertEquals(
          ":file/not-found",
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (throw (ex :file/not-found {})) "
                      + "(catch :socket/closed e :wrong) "
                      + "(catch :file/not-found e (:ex/code (ex-data e))))")
              .toString());
      assertEquals(
          ":file-error",
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (throw (ex :file/not-found {:ex/message \"missing\"})) "
                      + "(catch [:file/not-found :file/permission-denied] e :file-error))")
              .toString());
      assertEquals(
          "42",
          context.eval(HaraLanguage.ID, "(try (throw (ex :test/x {:ex/message \"x\"})) (catch e 42))").toString());
      assertEquals(
          ":failure/code",
          context.eval(HaraLanguage.ID, "(ex-message (ex :failure/code {}))").asString());
      assertEquals(
          "[1 1]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [exception (ex :test/provenance {})] "
                      + "(try (throw exception) "
                      + "(catch caught "
                      + "(let [provenance (ex-provenance caught)] "
                      + "[(:line (:ex/created-at provenance)) "
                      + "(count (:ex/throws provenance))]))))")
              .toString());
    }
  }

  @Test
  public void exceptionClassesArePortableAndNativeTypesAreDiagnostic() {
    try (Context context = context()) {
      assertEquals(
          ":ex.class/io",
          context.eval(HaraLanguage.ID, "(ex-class (ex :file/read {:ex/class :ex.class/io}))").toString());
      assertTrue(context.eval(HaraLanguage.ID, "(nil? (ex-class (ex :file/read {})))").asBoolean());
      assertEquals(
          ":ex.class/not-found",
          context.eval(HaraLanguage.ID, "(ex-class (ex :not-found {}))").toString());
      assertEquals(
          ":ex.class/internal",
          context.eval(HaraLanguage.ID, "(ex-class (ex :generic {}))").toString());
      assertEquals(
          ":hara/not-found",
          context.eval(HaraLanguage.ID, "(:ex/code (ex-data (ex :not-found {})))").toString());
      assertTrue(
          context.eval(HaraLanguage.ID, "(nil? (ex-native-type (ex :file/read {})))").asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(nil? (ex-native-type (ex-info \"legacy\" {:phase :test})))")
              .asBoolean());
      assertEquals(
          ":std.native.Exception",
          context.eval(HaraLanguage.ID, "(type (ex-info \"legacy\" {:phase :test}))").toString());
      assertEquals(
          "legacy",
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.native.Exception/message (ex-info \"legacy\" {:phase :test}))")
              .asString());
      assertEquals(
          "[:file/read \"missing\" :ex.class/io]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [error (ex :file/read {} :ex/message \"missing\" :ex/class :ex.class/io)] "
                      + "[(:ex/code (ex-data error)) (ex-message error) (ex-class error)])")
              .toString());
      PolyglotException malformed =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(ex :file/read {:ex/class :io})"));
      assertTrue(malformed.getMessage().contains(":ex/class must be a namespaced keyword"));
      PolyglotException ordinary =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(ex-native-type 42)"));
      assertTrue(ordinary.getMessage().contains("ex-native-type expects an Exception"));
    }
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .allowHostAccess(HostAccess.ALL)
            .allowHostClassLookup(name -> true)
            .build()) {
      context.eval(
          HaraLanguage.ID,
          "(ns exception-native-type (:flavor :jvm [java.lang RuntimeException]))");
      assertEquals(
          "java.lang.RuntimeException",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ex-native-type (new RuntimeException \"host\"))")
              .asString());
    }
  }

  private static Context context() {
    return Context.newBuilder(HaraLanguage.ID).allowIO(IOAccess.ALL).build();
  }

  private static Path specsRegistry() {
    return SpecRegistry.root();
  }
}
