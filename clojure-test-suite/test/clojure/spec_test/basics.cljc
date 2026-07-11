(ns clojure.spec-test.basics
  (:require [clojure.test :refer [deftest is testing]]
            [clojure.spec.alpha :as s]))

;; ── s/def, valid?, conform/unform ──────────────────────────────────────────

(deftest test-def-valid-conform
  (testing "predicate spec: valid?/conform/invalid?"
    (s/def ::even even?)
    (is (s/valid? ::even 4))
    (is (not (s/valid? ::even 3)))
    (is (= 4 (s/conform ::even 4)))
    (is (s/invalid? (s/conform ::even 3)))))

;; ── s/and threading ─────────────────────────────────────────────────────────

(deftest test-and-threading
  (testing "s/and short-circuits on first failing predicate"
    (is (s/valid? (s/and int? even?) 4))
    (is (not (s/valid? (s/and int? even?) 3)))
    (is (not (s/valid? (s/and int? even?) 4.0)))))

;; ── s/or tagging ────────────────────────────────────────────────────────────

(deftest test-or-tagging
  (testing "s/or conforms to a [tag value] pair for the first matching branch"
    (is (= [:i 5] (s/conform (s/or :i int? :s string?) 5)))
    (is (= [:s "x"] (s/conform (s/or :i int? :s string?) "x")))
    (is (s/invalid? (s/conform (s/or :i int? :s string?) :kw))))

  (testing "unform undoes the tagging"
    (is (= 5 (s/unform (s/or :i int? :s string?) [:i 5])))
    (is (= "x" (s/unform (s/or :i int? :s string?) [:s "x"])))))

;; ── set specs ────────────────────────────────────────────────────────────────

(deftest test-set-spec
  (testing "a literal set is usable directly as a spec (enum semantics)"
    (s/def ::color #{:red :green :blue})
    (is (s/valid? ::color :red))
    (is (not (s/valid? ::color :purple)))
    (is (= :green (s/conform ::color :green)))))

;; ── registry forward references ─────────────────────────────────────────────

(deftest test-registry-forward-reference
  (testing "a spec may reference another spec keyword before it is s/def'd"
    (s/def ::fwd-a ::fwd-b)
    (s/def ::fwd-b string?)
    (is (= "hi" (s/conform ::fwd-a "hi")))
    (is (s/invalid? (s/conform ::fwd-a 5)))))

;; ── explain-data problems shape ──────────────────────────────────────────────
;; Only assert on :path/:val/:via/:in, which are stable across our
;; implementation and upstream clojure.spec.alpha. Deliberately avoid
;; asserting on the :pred value itself (upstream qualifies predicate symbols
;; like `clojure.core/pos?`; this runtime records them unqualified as
;; written) and avoid comparing s/form output.

(deftest test-explain-data-problem-shape
  (testing "a single failing predicate reports path/val/via/in"
    (s/def ::pos-int (s/and int? pos?))
    (let [ed (s/explain-data ::pos-int -5)
          probs (:clojure.spec.alpha/problems ed)
          p (first probs)]
      (is (= 1 (count probs)))
      (is (= [] (:path p)))
      (is (= -5 (:val p)))
      (is (= [::pos-int] (:via p)))
      (is (= [] (:in p)))))

  (testing "a valid value explains to nil"
    (is (nil? (s/explain-data ::pos-int 5)))))
