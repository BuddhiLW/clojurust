(ns cljrs.lang-test.nth-arrays
  "`nth` indexes an array of every element kind, and stays narrower than
  `seqable?`: a map and a set are seqable and `nth` refuses them.

  The refusals are asserted next to the widening on purpose. `nth` lists the
  types it accepts rather than asking the shared seqable predicate, and this
  file is what pins that list from both sides."
  (:require [clojure.test :refer [deftest is testing]]))

(deftest nth-indexes-every-array-kind
  (testing "reference and integral arrays"
    (is (= :b (nth (object-array [:a :b]) 1)))
    (is (= 1 (nth (int-array [1 2]) 0)))
    (is (= 2 (nth (long-array [1 2]) 1)))
    (is (= 7 (nth (short-array [7 8]) 0)))
    (is (= 2 (nth (byte-array [1 2]) 1))))
  (testing "floating, boolean and char arrays"
    (is (= 1.5 (nth (float-array [1.5 2.5]) 0)))
    (is (= 2.5 (nth (double-array [1.5 2.5]) 1)))
    (is (false? (nth (boolean-array [true false]) 1)))
    (is (= \b (nth (char-array [\a \b]) 1)))))

(deftest nth-on-an-array-respects-the-bounds-rule
  (testing "out of range without a default is an error, as for a vector"
    (is (thrown? Exception (nth (int-array [1]) 5)))
    (is (thrown? Exception (nth (object-array []) 0))))
  (testing "out of range with a default is the default"
    (is (= :none (nth (int-array [1]) 5 :none)))
    (is (= :none (nth (int-array [1]) -1 :none)))
    (is (= :none (nth (int-array []) 0 :none)))))

(deftest nth-still-refuses-an-unordered-collection
  ;; Being seqable is not enough for nth, and widening it to arrays must not
  ;; widen it to these.
  (is (thrown? Exception (nth {:a 1} 0)))
  (is (thrown? Exception (nth #{1 2} 0))))
