(ns clojure.spec-test.instrument
  (:require [clojure.test :refer [deftest is testing]]
            [clojure.spec.alpha :as s]
            [clojure.spec.test.alpha :as stest]))

;; ── fdef + instrument catches a bad call ────────────────────────────────────

(defn spec-test-add [a b] (+ a b))
(s/fdef spec-test-add :args (s/cat :a int? :b int?) :ret int?)

(deftest test-instrument-catches-bad-call
  (testing "instrument wraps a var so calls are checked against its :args spec"
    (stest/instrument `spec-test-add)
    (is (= 5 (spec-test-add 2 3)))
    (is (thrown? Exception (spec-test-add 2 "x")))
    (stest/unstrument `spec-test-add)))

;; ── unstrument restores the raw fn ──────────────────────────────────────────

(defn spec-test-pair [a b] [a b])
(s/fdef spec-test-pair :args (s/cat :a int? :b int?))

(deftest test-unstrument-restores-raw-fn
  (testing "after unstrument, the raw (unchecked) fn runs with no :args check"
    (stest/instrument `spec-test-pair)
    (is (thrown? Exception (spec-test-pair 1 "x")))
    (stest/unstrument `spec-test-pair)
    (is (= [1 "x"] (spec-test-pair 1 "x")))))

;; ── with-instrument-disabled ─────────────────────────────────────────────────

(defn spec-test-listify [a b] (list a b))
(s/fdef spec-test-listify :args (s/cat :a int? :b int?))

(deftest test-with-instrument-disabled
  (testing "with-instrument-disabled bypasses the :args check for its extent"
    (stest/instrument `spec-test-listify)
    (is (thrown? Exception (spec-test-listify 1 "x")))
    (stest/with-instrument-disabled
      (is (= (list 1 "x") (spec-test-listify 1 "x"))))
    (stest/unstrument `spec-test-listify)))

;; ── s/assert ─────────────────────────────────────────────────────────────────

(deftest test-assert
  (testing "s/assert returns a valid value unchanged and throws on an invalid one"
    (s/def ::spec-test-pos-int (s/and int? pos?))
    (is (= 5 (s/assert ::spec-test-pos-int 5)))
    (is (thrown? Exception (s/assert ::spec-test-pos-int -5)))))
