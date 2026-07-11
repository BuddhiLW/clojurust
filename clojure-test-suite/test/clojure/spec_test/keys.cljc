(ns clojure.spec-test.keys
  (:require [clojure.test :refer [deftest is testing]]
            [clojure.spec.alpha :as s]))

;; ── s/keys :req / :opt (namespaced keys) ────────────────────────────────────

(s/def ::a int?)
(s/def ::b string?)

(deftest test-keys-req-opt
  (testing "required keys must be present and valid; optional keys are optional"
    (s/def ::m (s/keys :req [::a] :opt [::b]))
    (is (s/valid? ::m {::a 1}))
    (is (s/valid? ::m {::a 1 ::b "x"}))
    (is (not (s/valid? ::m {::b "x"})))        ; missing required key
    (is (not (s/valid? ::m {::a "no"})))       ; bad required value
    (is (not (s/valid? ::m {::a 1 ::b 2})))    ; bad optional value
    (is (not (s/valid? ::m 42)))))             ; not a map

(deftest test-keys-missing-req-invalid
  (testing "a map missing a required key is invalid, and conform reports it"
    (s/def ::needs-a (s/keys :req [::a]))
    (is (not (s/valid? ::needs-a {})))
    (is (s/invalid? (s/conform ::needs-a {})))
    (let [ed (s/explain-data ::needs-a {})]
      (is (pos? (count (:clojure.spec.alpha/problems ed))))
      (is (= [] (:path (first (:clojure.spec.alpha/problems ed))))))))

;; ── s/keys :req-un / :opt-un (unqualified keys) ─────────────────────────────

(deftest test-keys-un-variants
  (testing "req-un/opt-un validate against unqualified map keys"
    (s/def ::mu (s/keys :req-un [::a] :opt-un [::b]))
    (is (s/valid? ::mu {:a 1}))
    (is (s/valid? ::mu {:a 1 :b "x"}))
    (is (not (s/valid? ::mu {:b "x"})))       ; missing required unqualified key
    (is (not (s/valid? ::mu {:a "no"})))      ; bad value
    (is (not (s/valid? ::mu {:a 1 :b 2})))    ; bad optional value
    (is (= {:a 1} (s/conform ::mu {:a 1})))))

;; ── value conform through a registered spec ─────────────────────────────────

(deftest test-value-conforms-through-registered-spec
  (testing "conforming a plain value spec returns the value unchanged when valid"
    (is (= 1 (s/conform ::a 1)))
    (is (s/invalid? (s/conform ::a "not an int")))))

;; ── s/merge ──────────────────────────────────────────────────────────────────

(deftest test-merge
  (testing "s/merge is valid only when every constituent keys-spec is satisfied"
    (s/def ::ma (s/keys :req [::a]))
    (s/def ::mb (s/keys :req [::b]))
    (s/def ::mab (s/merge ::ma ::mb))
    (is (s/valid? ::mab {::a 1 ::b "x"}))
    (is (not (s/valid? ::mab {::a 1})))
    (is (= {::a 1 ::b "x"} (s/conform ::mab {::a 1 ::b "x"})))
    (is (pos? (count (:clojure.spec.alpha/problems (s/explain-data ::mab {::a 1})))))))
