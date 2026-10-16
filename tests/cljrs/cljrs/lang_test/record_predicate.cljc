(ns cljrs.lang-test.record-predicate
  "`record?` separates a defrecord instance from a deftype and a reify."
  (:require [clojure.test :refer [deftest is testing]]))

(defrecord Point [x y])

(deftype Pair [a b])

(defprotocol Described
  (describe [this]))

(deftest a-defrecord-instance-is-a-record
  (testing "the positional constructor"
    (is (record? (->Point 1 2))))
  (testing "the map constructor"
    (is (record? (map->Point {:x 1 :y 2})))))

(deftest a-deftype-instance-is-not-a-record
  (is (not (record? (->Pair 1 2)))))

(deftest a-reify-instance-is-not-a-record
  (is (not (record? (reify Described (describe [_] :r))))))

(deftest a-plain-map-is-not-a-record
  (is (not (record? {:x 1 :y 2}))))

(deftest non-collections-are-not-records
  (is (not (record? nil)))
  (is (not (record? 1)))
  (is (not (record? "s")))
  (is (not (record? [1 2]))))

(deftest assoc-on-a-record-returns-a-record
  (testing "a field, and a key the type never declared"
    (is (record? (assoc (->Point 1 2) :x 9)))
    (is (record? (assoc (->Point 1 2) :z 3)))))

(deftest a-record-keeps-its-fields-and-type
  (let [p (->Point 1 2)]
    (is (= 1 (:x p)))
    (is (= 9 (:x (assoc p :x 9))))
    (is (instance? Point p))))

(deftest metadata-does-not-change-what-a-record-is
  (testing "the wrapper is not part of the type"
    (is (record? (with-meta (->Point 1 2) {:k 1})))
    (is (not (record? (with-meta {:x 1} {:k 1}))))))

;; ── which datatype forms carry metadata ─────────────────────────────────────
;;
;; Three forms, and metadata splits them 2-1 rather than the way `record?`
;; does: `defrecord` and `reify` accept it, `deftype` refuses. On the JVM the
;; refusal is a ClassCastException, because a `deftype` class implements
;; neither IObj nor IMeta; cljrs raises its own wrong-type error. Both are
;; `Exception`, which is all these assertions need.
;;
;; The 2-1 split is the point. Any runtime that decides this from a single
;; "is it a record" flag gets `reify` wrong in whichever direction it picked.

(deftest a-record-carries-metadata
  (is (= {:k 1} (meta (with-meta (->Point 1 2) {:k 1})))))

(deftest a-reify-carries-metadata
  (let [r (reify Described (describe [_] :r))]
    (is (= {:k 1} (meta (with-meta r {:k 1}))))
    (testing "and still answers its protocol through the wrapper"
      (is (= :r (describe (with-meta r {:k 1})))))))

(deftest a-deftype-refuses-metadata
  (testing "clojure.core's deftype implements no metadata interface, so a
            dialect that accepted this would let code compile that cannot port"
    (is (thrown? Exception (with-meta (->Pair 1 2) {:k 1})))))
