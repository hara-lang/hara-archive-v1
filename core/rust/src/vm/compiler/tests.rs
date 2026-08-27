use super::compile_source;
use crate::Runtime;

#[test]
fn cond_compiles_a_terminating_loop_branch() {
    compile_source("(loop [i 0] (cond (< i 2) (recur (+ i 1)) :else i))")
        .expect("cond with a terminating branch compiles");
}

#[test]
fn short_circuit_forms_compile_with_a_terminating_final_operand() {
    compile_source("(fn [x] (and x (throw \"and\")))")
        .expect("and short-circuit path reaches the function return");
    compile_source("(fn [x] (or x (throw \"or\")))")
        .expect("or short-circuit path reaches the function return");
}

#[test]
fn structural_callable_compiles_as_a_first_class_value() {}

#[test]
fn map_literals_preserve_source_order_in_bytecode() {
    let mut runtime = Runtime::core();
    assert_eq!(
        runtime
            .eval_bytecode_native("{:tag :invalid :value 3}")
            .unwrap(),
        "{:tag :invalid :value 3}"
    );
}

#[test]
fn quote_compiles_to_literal_bytecode_values() {
    let mut runtime = Runtime::core();
    assert_eq!(runtime.eval_bytecode_native("(quote x)").unwrap(), "x");
    assert_eq!(
        runtime.eval_bytecode_native("(quote [x 1])").unwrap(),
        "[x 1]"
    );
}

#[test]
fn a_declared_global_wins_over_a_reserved_operator_name() {
    let mut runtime = Runtime::core();
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "(defn await
                   ([value] (await value 2))
                   ([value extra] (+ value extra)))
                 (await 40)"
            )
            .unwrap(),
        "42"
    );
}

#[test]
fn macro_introduced_lexical_bindings_are_not_captures() {
    let mut runtime = Runtime::core();
    runtime
        .eval_native(
            "(defmacro if-let [binding then alternative]
               (let [name (nth binding 0)
                     expression (nth binding 1)]
                 `(let [~name ~expression]
                    (if ~name ~then ~alternative))))",
        )
        .unwrap();
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "(defn invoke-selected [candidate]
                   (if-let [selected candidate]
                     (selected 41)
                     0))
                 (invoke-selected inc)"
            )
            .unwrap(),
        "42"
    );
}

#[test]
fn destructuring_lowers_across_bindings_parameters_and_recur() {
    let mut runtime = Runtime::core();
    runtime.use_namespace("std.foundation");
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "(let [[a b & more :as all] [1 2 3 4]
                        {:keys [x] :or {x 9} :as m} {:x 5}]
                   [a b more all x m])"
            )
            .unwrap(),
        "[1 2 [3 4] [1 2 3 4] 5 {:x 5}]"
    );
    assert_eq!(
        runtime
            .eval_bytecode_native("((fn [[a b] {:keys [x]}] (+ a b x)) [1 2] {:x 3})")
            .unwrap(),
        "6"
    );
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "((fn [{:keys [callback]
                        :or {callback (fn [value] value)}}]
                    (callback 41))
                  {})"
            )
            .unwrap(),
        "41"
    );
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "(map (fn [{:keys [name optional?]}]
                        [name optional?])
                      [{:name \"id\" :optional? true}])"
            )
            .unwrap(),
        "[[\"id\" true]]"
    );
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "(defn parts [form]
                   (let [[_ head & tail] form]
                     (if (symbol? head)
                       [head (first tail) (rest tail)]
                       [nil head tail])))
                 (defn compatible? [form]
                   (let [[name] (parts form)]
                     (nil? name)))
                 (compatible? '(fn [x] x))"
            )
            .unwrap(),
        "true"
    );
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "(loop [[head & tail] [1 2 3] out []]
                   (if head
                     (recur tail (conj out head))
                     out))"
            )
            .unwrap(),
        "[1 2 3]"
    );
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "(letfn [(even* [n] (if (= n 0) true (odd* (- n 1))))
                          (odd* [n] (if (= n 0) false (even* (- n 1))))]
                   [(even* 10) (odd* 9)])"
            )
            .unwrap(),
        "[true true]"
    );
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "(= (let [require-fields (fn [source fields]
                          (reduce (fn [out field]
                                    (if (nil? (get source field)) out out))
                                  source
                                  fields))
                       profile (fn [value]
                                 (require-fields value
                                                 [:profile/id
                                                  :profile/version
                                                  :profile/operators]))]
                   (profile {:profile/id :dsl
                             :profile/version 1
                             :profile/operators {:a 1}}))
                   {:profile/id :dsl
                    :profile/version 1
                    :profile/operators {:a 1}})"
            )
            .unwrap(),
        "true"
    );
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "(do (defn __gate-require-fields [source fields]
                   (reduce (fn [out field]
                             (if (nil? (get source field)) out out))
                           source
                           fields))
                 (defn __gate-profile [value]
                   (__gate-require-fields value
                                          [:profile/id
                                           :profile/version
                                           :profile/operators]))
                 (= (__gate-profile {:profile/id :dsl
                                  :profile/version 1
                                  :profile/operators {:a 1}})
                   {:profile/id :dsl
                    :profile/version 1
                    :profile/operators {:a 1}}))"
            )
            .unwrap(),
        "true"
    );

    runtime.register_resource(
        "__gate.grammar",
        "(ns __gate.grammar)
         (defn fail [message data]
           (throw (ex-info message data)))
         (defn require-fields [source fields]
           (reduce (fn [out field]
                     (if (nil? (get source field))
                       (fail \"Missing grammar source field\"
                             {:field field :source source})
                       out))
                   source
                   fields))
         (defn source [kind id version value]
           (if (or (nil? id) (not (number? version)) (<= version 0))
             (fail \"Invalid grammar source identity\"
                   {:kind kind :id id :version version})
             (assoc value
                    :source/kind kind
                    :source/id id
                    :source/version version)))
         (defn profile [value]
           (source :profile
                   (:profile/id value)
                   (:profile/version value)
                   (require-fields value
                                   [:profile/id
                                    :profile/version
                                    :profile/operators])))",
    );
    runtime
        .eval_text("(ns __gate.consumer (:require [__gate.grammar :as grammar]))")
        .unwrap();
    runtime.eval_text("(ns __gate.grammar)").unwrap();
    runtime
        .eval_bytecode_native(
            "(defn fail [message data]
               (throw (ex-info message data)))
             (defn require-fields [source fields]
               (reduce (fn [out field]
                         (if (nil? (get source field))
                           (fail \"Missing grammar source field\"
                                 {:field field :source source})
                           out))
                       source
                       fields))
             (defn source [kind id version value]
               (if (or (nil? id) (not (number? version)) (<= version 0))
                 (fail \"Invalid grammar source identity\"
                       {:kind kind :id id :version version})
                 (assoc value
                        :source/kind kind
                        :source/id id
                        :source/version version)))
             (defn profile [value]
               (source :profile
                       (:profile/id value)
                       (:profile/version value)
                       (require-fields value
                                       [:profile/id
                                        :profile/version
                                        :profile/operators])))",
        )
        .unwrap();
    runtime.use_namespace("__gate.consumer");
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "(= (grammar/profile {:profile/id :dsl
                                   :profile/version 1
                                   :profile/operators {:a 1}})
                   {:profile/id :dsl
                    :profile/version 1
                    :profile/operators {:a 1}
                    :source/kind :profile
                    :source/id :dsl
                    :source/version 1})"
            )
            .unwrap(),
        "true"
    );
}

#[test]
fn destructuring_generated_calls_ignore_shadowing() {
    let mut runtime = Runtime::core();
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "(ns compiler.destructure.override
                   (:config {:override [atom deref drop get reset!]}))
                 (defn atom [value] :local-atom)
                 (defn deref [value] :local-deref)
                 (defn drop [amount value] :local-drop)
                 (defn get [value key] :local-get)
                 (defn reset! [value next] :local-reset)
                 [(let [[head & tail] [1 2 3]] [head tail])
                  (let [{:keys [value]} {:value 42}] value)
                  (letfn [(even* [n] (if (= n 0) true (odd* (- n 1))))
                          (odd* [n] (if (= n 0) false (even* (- n 1))))]
                    [(even* 4) (odd* 3)])]"
            )
            .unwrap(),
        "[[1 [2 3]] 42 [true true]]"
    );
}

#[test]
fn static_array_calls_compile_to_native_bytecode() {
    let mut runtime = Runtime::core();
    assert_eq!(
        runtime
            .eval_bytecode_native(
                "(defn mutate-and-clone [array value]
                   (Arr/push-last array value)
                   (Base/vec (Arr/clone array)))
                 (mutate-and-clone (array 1 2) 3)"
            )
            .unwrap(),
        "[1 2 3]"
    );
}
