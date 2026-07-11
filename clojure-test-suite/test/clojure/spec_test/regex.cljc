(ns clojure.spec-test.regex
  (:require [clojure.test :refer [deftest is testing]]
            [clojure.spec.alpha :as s]))

;; ── s/cat ────────────────────────────────────────────────────────────────────

(deftest test-cat-basic
  (testing "cat conforms a sequential into a tagged map"
    (is (= {:a 1 :b "x"} (s/conform (s/cat :a int? :b string?) [1 "x"])))
    (is (s/invalid? (s/conform (s/cat :a int? :b string?) [1 2])))   ; wrong type
    (is (s/invalid? (s/conform (s/cat :a int? :b string?) [1])))     ; too short
    (is (s/invalid? (s/conform (s/cat :a int?) [1 2])))              ; too long
    (is (s/invalid? (s/conform (s/cat :a int?) 5)))))                ; not sequential

;; ── nesting + splicing ───────────────────────────────────────────────────────

(deftest test-cat-nesting-and-splicing
  (testing "a nested regex op splices its elements into the enclosing sequence"
    (is (= {:a 1 :b ["x" "y"]}
           (s/conform (s/cat :a int? :b (s/* string?)) [1 "x" "y"])))))

(deftest test-keyword-ref-splicing
  (testing "a keyword ref to a regex spec splices; to a non-regex spec it consumes one element"
    (s/def ::regex-ref-int int?)
    (is (= {:x 1 :y "x"}
           (s/conform (s/cat :x ::regex-ref-int :y string?) [1 "x"])))
    (s/def ::regex-ref-star (s/* int?))
    (is (= {:nums [1 2] :tail "x"}
           (s/conform (s/cat :nums ::regex-ref-star :tail string?) [1 2 "x"])))))

;; ── s/spec as a nesting boundary ─────────────────────────────────────────────

(deftest test-spec-boundary
  (testing "s/spec wraps a regex op so it consumes exactly one (nested) element"
    (is (= {:nums [1 2] :tail "x"}
           (s/conform (s/cat :nums (s/spec (s/* int?)) :tail string?)
                      [[1 2] "x"])))))

;; ── s/alt / s/* / s/+ / s/? ──────────────────────────────────────────────────

(deftest test-alt-star-plus-maybe
  (testing "alt tags its matching branch"
    (is (= [:i 5] (s/conform (s/alt :i int? :s string?) [5])))
    (is (s/invalid? (s/conform (s/alt :i int? :s string?) [:kw]))))

  (testing "* matches zero or more"
    (is (= [] (s/conform (s/* int?) [])))
    (is (s/valid? (s/* int?) []))
    (is (= [1 2 3] (s/conform (s/* int?) [1 2 3])))
    (is (s/invalid? (s/conform (s/* int?) [1 :a 3]))))

  (testing "+ requires at least one"
    (is (= [1] (s/conform (s/+ int?) [1])))
    (is (s/invalid? (s/conform (s/+ int?) []))))

  (testing "? is optional"
    (is (= 5 (s/conform (s/? int?) [5])))
    (is (nil? (s/conform (s/? int?) [])))
    (is (s/valid? (s/? int?) []))
    (is (s/invalid? (s/conform (s/? int?) [1 2])))))

;; ── s/& post-predicates ──────────────────────────────────────────────────────

(deftest test-amp-post-predicate
  (testing "& applies an additional predicate to the conformed regex result"
    (let [even-count? (fn [xs] (even? (count xs)))]
      (is (= [1 2] (s/conform (s/& (s/* int?) even-count?) [1 2])))
      (is (s/invalid? (s/conform (s/& (s/* int?) even-count?) [1 2 3]))))))

;; ── insufficient / extra input via explain-data :reason ─────────────────────
;; These :reason strings are the same ones upstream clojure.spec.alpha uses.

(deftest test-explain-reasons
  (testing "premature end of input reports \"Insufficient input\""
    (let [p (first (:clojure.spec.alpha/problems
                     (s/explain-data (s/cat :a int? :b string?) [1])))]
      (is (= "Insufficient input" (:reason p)))
      (is (= [:b] (:path p)))))

  (testing "leftover input reports \"Extra input\""
    (let [p (first (:clojure.spec.alpha/problems
                     (s/explain-data (s/cat :a int?) [1 2])))]
      (is (= "Extra input" (:reason p)))
      (is (= [1] (:in p)))))

  (testing "an element that fails its predicate reports path/in/val"
    (let [p (first (:clojure.spec.alpha/problems
                     (s/explain-data (s/cat :a int? :b string?) [1 :bad])))]
      (is (= [:b] (:path p)))
      (is (= [1] (:in p)))
      (is (= :bad (:val p))))))

;; ── conform/unform round-trips ───────────────────────────────────────────────

(deftest test-regex-unform-roundtrips
  (testing "unform inverts conform for every regex op"
    (let [even-count? (fn [xs] (even? (count xs)))
          cases [[(s/cat :a int? :b string?) [1 "x"]]
                 [(s/cat :a int? :b (s/* string?)) [1 "x" "y"]]
                 [(s/alt :i int? :s string?) [5]]
                 [(s/* int?) [1 2 3]]
                 [(s/* int?) []]
                 [(s/+ int?) [1 2]]
                 [(s/? int?) [5]]
                 [(s/? int?) []]
                 [(s/& (s/* int?) even-count?) [1 2]]]]
      (doseq [[spec input] cases]
        (is (= input (vec (s/unform spec (s/conform spec input)))))))))
