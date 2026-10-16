(ns cljrs.lang-test.protocol-impl
  "A protocol named in an impl position may be qualified, and resolves through
  its own namespace. Each of the five impl sites is asserted separately."
  (:require [clojure.test :refer [deftest is testing]]
            [cljrs.lang-test.fixture.proto :as mp]))

;; ── The five impl sites, protocol reached through an alias ───────────────────

(defrecord AliasRecord [n]
  mp/IThing
  (-describe [_] [:record n]))

(deftest defrecord-implements-a-protocol-from-another-namespace
  (testing "an inline defrecord impl resolves the aliased protocol"
    (is (= [:record 1] (mp/-describe (->AliasRecord 1))))))

(deftype AliasType [n]
  mp/IThing
  (-describe [_] [:type n]))

(deftest deftype-implements-a-protocol-from-another-namespace
  (testing "an inline deftype impl resolves the aliased protocol"
    (is (= [:type 2] (mp/-describe (->AliasType 2))))))

(deftest reify-implements-a-protocol-from-another-namespace
  (testing "reify resolves the aliased protocol"
    (is (= :reified (mp/-describe (reify mp/IThing (-describe [_] :reified)))))))

(deftest extend-type-names-a-protocol-from-another-namespace
  (testing "extend-type resolves the aliased protocol"
    (extend-type String
      mp/IThing
      (-describe [s] [:string s]))
    (is (= [:string "hi"] (mp/-describe "hi")))))

(deftest extend-protocol-names-a-protocol-from-another-namespace
  (testing "extend-protocol resolves the aliased protocol"
    (extend-protocol mp/IThing
      Long
      (-describe [n] [:long n]))
    (is (= [:long 7] (mp/-describe 7)))))

;; ── Fully qualified, no alias ────────────────────────────────────────────────

(defrecord QualifiedRecord []
  cljrs.lang-test.fixture.proto/IThing
  (-describe [_] :qualified))

(deftest a-fully-qualified-protocol-name-resolves-without-an-alias
  (testing "the alias is a convenience, not the mechanism"
    (is (= :qualified
           (cljrs.lang-test.fixture.proto/-describe (->QualifiedRecord))))))

;; ── The unqualified case still resolves in the current ns ────────────────────

(defprotocol Local
  (-local-describe [this]))

(defrecord LocalRecord []
  Local
  (-local-describe [_] :same-ns))

(deftest an-unqualified-protocol-name-still-resolves-in-the-current-ns
  (testing "fixing the qualified path did not break the unqualified one"
    (is (= :same-ns (-local-describe (->LocalRecord))))))
