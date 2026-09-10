(ns cljrs.lang-test.meta-transparency
  "Property oracle: for every special form, evaluating `^meta F` agrees with
  evaluating `F`.

  Reader metadata is advisory for structural dispatch — a hint in front of a
  params vector, a field vector, a binding vector, a name symbol or an arity
  clause must not change how the form is parsed. Stated as a property rather
  than as a table of expected values, so a new annotation site costs one
  template.

  The three strata are kept apart on purpose: `annotations` and the templates
  are data, `render` is a calculation over strings, and `value-of` is the only
  thing here that runs anything."
  (:require [clojure.test :refer [deftest is testing]]
            [clojure.string :as str]))

;; ── Data ─────────────────────────────────────────────────────────────────────

(def ^:private annotations
  "Every annotation that must be structurally transparent."
  ["^:marker " "^String " "^{:doc \"d\"} "])

;; ── Calculation ──────────────────────────────────────────────────────────────

(defn- render
  "`template` carries one `{}` placeholder marking the annotation site."
  [template annotation]
  (str/replace template "{}" annotation))

;; ── Boundary ─────────────────────────────────────────────────────────────────

(defn- value-of [src]
  (eval (read-string (str "(do " src ")"))))

(defn- disagreements
  "The annotations under which `template` stops agreeing with its bare form.
  Empty means the site is transparent."
  [template]
  (let [expected (value-of (render template ""))]
    (vec (for [annotation annotations
               :let [actual (value-of (render template annotation))]
               :when (not= actual expected)]
           {:annotation annotation :expected expected :actual actual}))))

;; ── def / defn ───────────────────────────────────────────────────────────────

(deftest def-name
  (testing "a hint on the name being defined"
    (is (= [] (disagreements "(def {}mt-x 41) (inc mt-x)")))))

(deftest defn-name
  (testing "a hint on the function name"
    (is (= [] (disagreements "(defn {}mt-f [a] (inc a)) (mt-f 41)")))))

(deftest defn-params-vector
  (testing "a return hint before the params vector"
    ;; `(defn f ^String [s] s)` — green on the JVM, once rejected here with
    ;; "fn* expects vector or arity clauses".
    (is (= [] (disagreements "(defn mt-f {}[a] (inc a)) (mt-f 41)")))))

(deftest defn-docstring-and-body
  (testing "a hint after a docstring"
    (is (= [] (disagreements "(defn mt-f \"doc\" {}[a] (inc a)) (mt-f 41)")))))

;; ── fn ───────────────────────────────────────────────────────────────────────

(deftest fn-params-vector
  (testing "a hint before an anonymous fn's params"
    (is (= [] (disagreements "((fn {}[a] (inc a)) 41)")))))

(deftest fn-arity-clause
  (testing "a hint before the first arity clause of a multi-arity fn"
    (is (= [] (disagreements "((fn {}([a] (inc a)) ([a b] b)) 41)")))))

(deftest fn-param-symbol
  (testing "a hint on a parameter symbol"
    (is (= [] (disagreements "((fn [{}a] (inc a)) 41)")))))

;; ── Binding forms ────────────────────────────────────────────────────────────

(deftest let-binding-vector
  (testing "a hint before a let binding vector"
    (is (= [] (disagreements "(let {}[a 42] a)")))))

(deftest loop-binding-vector
  (testing "a hint before a loop binding vector"
    (is (= [] (disagreements "(loop {}[a 0] (if (< a 42) (recur (inc a)) a))")))))

(deftest letfn-binding-vector
  (testing "a hint before a letfn binding vector"
    (is (= [] (disagreements "(letfn {}[(g [x] (inc x))] (g 41))")))))

(deftest letfn-binding-name
  (testing "a hint on a letfn binding name"
    (is (= [] (disagreements "(letfn [({}g [x] (inc x))] (g 41))")))))

(deftest binding-vector
  (testing "a hint before a binding vector"
    (is (= [] (disagreements
               "(def ^:dynamic *mt-v* 1) (binding {}[*mt-v* 42] *mt-v*)")))))

;; ── defrecord ────────────────────────────────────────────────────────────────

(deftest defrecord-field
  (testing "a hint on a record field — the reported repro"
    (is (= [] (disagreements "(defrecord MtR [{}x]) (:x (->MtR 42))")))))

(deftest defrecord-name-and-field-vector
  (testing "a hint on the record name, and on its field vector"
    (is (= [] (disagreements "(defrecord {}MtR [x]) (:x (->MtR 42))")))
    (is (= [] (disagreements "(defrecord MtR {}[x]) (:x (->MtR 42))")))))

;; ── Dispatch forms ───────────────────────────────────────────────────────────

(deftest defmulti-and-defmethod
  (testing "a hint on the multimethod name, and on a method's params"
    (is (= [] (disagreements
               "(defmulti {}mt-m identity) (defmethod mt-m 1 [_] 42) (mt-m 1)")))
    (is (= [] (disagreements
               "(defmulti mt-m identity) (defmethod mt-m 1 {}[_] 42) (mt-m 1)")))))

(deftest defprotocol-and-extend-type
  (testing "a hint on the protocol name, the extended type, and a method name"
    (is (= [] (disagreements
               "(defprotocol {}MtP (mt-pm [this])) (defrecord MtR [x]) (extend-type MtR MtP (mt-pm [this] (:x this))) (mt-pm (->MtR 42))")))
    (is (= [] (disagreements
               "(defprotocol MtP (mt-pm [this])) (defrecord MtR [x]) (extend-type {}MtR MtP (mt-pm [this] (:x this))) (mt-pm (->MtR 42))")))
    (is (= [] (disagreements
               "(defprotocol MtP (mt-pm [this])) (defrecord MtR [x]) (extend-type MtR MtP ({}mt-pm [this] (:x this))) (mt-pm (->MtR 42))")))))

;; Named for the site, not for the macro: a `deftest` interns its name, and
;; `extend-protocol` would shadow `clojure.core/extend-protocol` in this
;; namespace — which the JVM warns about and which the templates below then
;; depend on NOT having happened.
(deftest extend-protocol-site
  (testing "a hint on the protocol named by extend-protocol"
    (is (= [] (disagreements
               "(defprotocol MtP (mt-pm [this])) (defrecord MtR [x]) (extend-protocol {}MtP MtR (mt-pm [this] (:x this))) (mt-pm (->MtR 42))")))))

(deftest defrecord-inline-impl
  (testing "a hint on a protocol named in a defrecord impl position"
    (is (= [] (disagreements
               "(defprotocol MtP (mt-pm [this])) (defrecord MtR [x] {}MtP (mt-pm [this] (:x this))) (mt-pm (->MtR 42))")))))

(deftest reify-impl
  (testing "a hint on a protocol named in a reify impl position"
    (is (= [] (disagreements
               "(defprotocol MtP (mt-pm [this])) (mt-pm (reify {}MtP (mt-pm [this] 42)))")))))

;; ── Macros and conditions ────────────────────────────────────────────────────

(deftest defmacro-params-vector
  (testing "a hint before a macro's params vector"
    (is (= [] (disagreements "(defmacro mt-mac {}[a] (list 'inc a)) (mt-mac 41)")))))

(deftest pre-post-conditions
  (testing "a hint before a pre/post condition map"
    (is (= [] (disagreements "(defn mt-f [a] {}{:pre [(pos? a)]} (inc a)) (mt-f 41)")))))
