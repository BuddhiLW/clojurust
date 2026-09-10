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
