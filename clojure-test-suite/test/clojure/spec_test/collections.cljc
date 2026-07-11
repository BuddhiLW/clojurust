(ns clojure.spec-test.collections
  (:require [clojure.test :refer [deftest is testing]]
            [clojure.spec.alpha :as s]))

;; ── coll-of ──────────────────────────────────────────────────────────────────

(deftest test-coll-of-basic
  (testing "coll-of conforms every element and rebuilds the collection"
    (is (= [1 2 3] (s/conform (s/coll-of int?) [1 2 3])))
    (is (s/invalid? (s/conform (s/coll-of int?) [1 "x"])))))

(deftest test-coll-of-kind
  (testing ":kind restricts the accepted collection type"
    (is (s/invalid? (s/conform (s/coll-of int? :kind vector?) '(1 2 3))))
    (is (s/valid? (s/coll-of int? :kind vector?) [1 2 3]))))

(deftest test-coll-of-min-max-count
  (testing ":min-count / :max-count bound the collection size"
    (is (s/invalid? (s/conform (s/coll-of int? :min-count 3) [1 2])))
    (is (s/valid? (s/coll-of int? :min-count 3) [1 2 3]))
    (is (s/invalid? (s/conform (s/coll-of int? :max-count 2) [1 2 3])))
    (is (s/valid? (s/coll-of int? :max-count 2) [1 2]))))

(deftest test-coll-of-distinct
  (testing ":distinct requires all elements be unique"
    (is (= [1 2 3] (s/conform (s/coll-of int? :distinct true) [1 2 3])))
    (is (s/invalid? (s/conform (s/coll-of int? :distinct true) [1 1 2])))))

(deftest test-coll-of-into
  (testing ":into controls the output collection type"
    (is (= #{1 2 3} (s/conform (s/coll-of int? :into #{}) [1 2 3])))
    (is (= '(1 2 3) (s/conform (s/coll-of int?) '(1 2 3))))))

;; ── map-of ───────────────────────────────────────────────────────────────────

(deftest test-map-of-basic
  (testing "map-of validates keys and values; original keys are kept by default"
    (is (= {:a 1 :b 2} (s/conform (s/map-of keyword? int?) {:a 1 :b 2})))
    (is (s/invalid? (s/conform (s/map-of keyword? int?) {:a "x"})))
    (is (s/invalid? (s/conform (s/map-of keyword? int?) {"not-kw" 1})))))

(deftest test-map-of-conform-keys
  (testing ":conform-keys true swaps in the conformed key"
    (is (= {"a" 1}
           (s/conform (s/map-of (s/conformer name) int? :conform-keys true) {:a 1})))))

;; ── every vs coll-of ─────────────────────────────────────────────────────────

(deftest test-every-vs-coll-of
  (testing "coll-of conforms (and tags) every element; every validates only"
    (let [elem (s/or :i int? :s string?)]
      (is (= [[:i 1] [:s "x"]] (s/conform (s/coll-of elem) [1 "x"])))
      (is (= [1 "x"] (s/conform (s/every elem) [1 "x"])))
      (is (s/invalid? (s/conform (s/every elem) [1 :bad])))))

  (testing "every-kv validates both key and value but never rebuilds"
    (is (= {:a 1} (s/conform (s/every-kv keyword? int?) {:a 1})))
    (is (s/invalid? (s/conform (s/every-kv keyword? int?) {:a "x"})))))

;; ── tuple ────────────────────────────────────────────────────────────────────

(deftest test-tuple
  (testing "tuple validates a fixed-length, positionally-typed vector"
    (is (= [1 "x"] (s/conform (s/tuple int? string?) [1 "x"])))
    (is (s/invalid? (s/conform (s/tuple int? string?) [1])))
    (is (s/invalid? (s/conform (s/tuple int? string?) 5))))

  (testing "explain-data reports the failing index in :path and :in"
    (let [p (first (:clojure.spec.alpha/problems
                     (s/explain-data (s/tuple int? string?) [1 :bad])))]
      (is (= [1] (:path p)))
      (is (= [1] (:in p)))
      (is (= :bad (:val p)))))

  (testing "unform round-trips"
    (let [spec (s/tuple int? string?)]
      (is (= [1 "x"] (s/unform spec (s/conform spec [1 "x"])))))))

;; ── nilable ──────────────────────────────────────────────────────────────────

(deftest test-nilable
  (testing "nilable accepts nil or the wrapped spec"
    (is (= nil (s/conform (s/nilable int?) nil)))
    (is (= 5 (s/conform (s/nilable int?) 5)))
    (is (s/invalid? (s/conform (s/nilable int?) "x")))))

;; ── int-in / double-in ───────────────────────────────────────────────────────

(deftest test-int-in
  (testing "int-in bounds an integer with an exclusive end"
    (is (s/valid? (s/int-in 0 10) 5))
    (is (not (s/valid? (s/int-in 0 10) 10)))
    (is (not (s/valid? (s/int-in 0 10) 5.0)))))

(deftest test-double-in
  (testing "double-in bounds a double, with optional NaN/infinite checks"
    (is (s/valid? (s/double-in :min 0.0 :max 10.0) 5.0))
    (is (not (s/valid? (s/double-in :min 0.0 :max 10.0) 20.0)))
    (let [nan (/ 0.0 0.0)
          pos-inf (/ 1.0 0.0)]
      (is (not (s/valid? (s/double-in :NaN? false) nan)))
      (is (s/valid? (s/double-in) nan))
      (is (not (s/valid? (s/double-in :infinite? false) pos-inf)))
      (is (s/valid? (s/double-in) pos-inf)))))

;; ── multi-spec ───────────────────────────────────────────────────────────────

(defmulti event-type :event/type)

(s/def :event/type keyword?)
(s/def :event/created (s/keys :req [:event/type]))
(defmethod event-type :created [_] :event/created)

(s/def :event/event (s/multi-spec event-type :event/type))

(deftest test-multi-spec
  (testing "multi-spec dispatches conform to the branch registered for the tag"
    (is (= {:event/type :created} (s/conform :event/event {:event/type :created})))
    (is (s/invalid? (s/conform :event/event {:event/type :unknown})))))

;; ── conformer ────────────────────────────────────────────────────────────────

(deftest test-conformer
  (testing "conformer transforms the value on conform"
    (is (= 10 (s/conform (s/conformer #(* 2 %)) 5))))

  (testing "conformer threads its transformed value through the rest of s/and"
    (is (= 10 (s/conform (s/and int? (s/conformer #(* 2 %))) 5))))

  (testing "unform applies the given unf fn, or is identity without one"
    (is (= 5 (s/unform (s/conformer #(* 2 %) #(/ % 2)) 10)))
    (is (= 10 (s/unform (s/conformer #(* 2 %)) 10)))))
