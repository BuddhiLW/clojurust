(ns clojure.core-test.val
  (:require [clojure.test :refer [are deftest is testing]]
            [clojure.core-test.portability #?(:cljs :refer-macros :default :refer) [when-var-exists] :as p]))

(when-var-exists val
  (deftest test-val
    (testing "basic tests"
      (is (nil? (val (first {nil nil}))))
      (is (= :v (val (first {:k :v}))))
      (is (contains? #{:two :v} (val (first {:k :v, :one :two}))))
      ;; Note: the following may be built on shaky ground, per Rich:
      ;; https://groups.google.com/g/clojure/c/FVcrbHJpCW4/m/Fh7NsX_Yb7sJ
      (is (= 'v (val #?(:cljs    (cljs.core/MapEntry. 'k 'v nil)
                        :lpy     (map-entry 'k 'v)
                        :rust    (map-entry 'k 'v)
                        :default (clojure.lang.MapEntry/create 'k 'v)))))
      (is (= :v (val (first (hash-map :k :v)))))
      (when-var-exists sorted-map
        (is (= :v (val (first (sorted-map :k :v))))))
      (when-var-exists array-map
        (is (= :v (val (first (array-map :k :v)))))))
    (testing "`val` throws on lots of things"
      (are [arg] (p/thrown? (val arg))
        0
        #?@(:jank [] ; ; jank just returns the result of `second`, since it assumes a map entry is a vector of two objects.
            :rust [nil
                   '()
                   '(1 2)
                   {}
                   {1 2}
                   []
                   [1 2]
                   #{}
                   #{1 2}]
            :default [nil
                      '()
                      '(1 2)
                      {}
                      {1 2}
                      []
                      [1 2]
                      #{}
                      #{1 2}])))))
