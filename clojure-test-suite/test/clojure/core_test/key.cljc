(ns clojure.core-test.key
  (:require [clojure.test :refer [are deftest is testing]]
            [clojure.core-test.portability #?(:cljs :refer-macros :default :refer) [when-var-exists] :as p]))

(when-var-exists key
  (deftest test-key
    (testing "basic tests"
      (is (= nil (key (first {nil nil}))))
      (is (= :k (key (first {:k :v}))))
      (is (contains? #{:k :one} (key (first {:k :v, :one :two}))))
      ;; Note: the following may be built on shaky ground, per Rich:
      ;; https://groups.google.com/g/clojure/c/FVcrbHJpCW4/m/Fh7NsX_Yb7sJ
      (is (= 'k (key #?(:cljs    (cljs.core/MapEntry. 'k 'v nil)
                        :lpy     (map-entry 'k 'v)
                        :rust    (map-entry 'k 'v)
                        :default (clojure.lang.MapEntry/create 'k 'v)))))
      (is (= :k (key (first (hash-map :k :v)))))
      (when-var-exists sorted-map
        (is (= :k (key (first (sorted-map :k :v))))))
      (when-var-exists array-map
        (is (= :k (key (first (array-map :k :v)))))))
    (testing "`key` throws on lots of things"
      (are [arg] (p/thrown? (key arg))
        0
        #?@(:jank [] ; jank just returns the result of `first`, since it assumes a map entry is a vector of two objects.
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
