(ns cljrs.lang-test.reify-capture
  "Each evaluation of one reify form yields an instance that closes over the
  locals of THAT evaluation; a later evaluation does not change an earlier
  instance."
  (:require [clojure.test :refer [deftest is testing]]))

(defprotocol Getter
  (-get [this])
  (-get-with [this x]))

(defn make-getter [k]
  (reify Getter
    (-get [_] k)
    (-get-with [_ x] [k x])))

(deftest one-form-two-instances-keep-their-own-locals
  (let [a (make-getter :a)
        b (make-getter :b)]
    (testing "the first instance is not overwritten by the second"
      (is (= :a (-get a)))
      (is (= :b (-get b))))
    (testing "a method with extra params sees its own capture"
      (is (= [:a 1] (-get-with a 1)))
      (is (= [:b 2] (-get-with b 2))))))

(defn make-from-let [n]
  (let [doubled (* 2 n)
        label   (str "n=" n)]
    (reify Getter
      (-get [_] [doubled label])
      (-get-with [_ x] (+ doubled x)))))

(deftest let-locals-are-captured-per-evaluation
  (let [xs (mapv make-from-let [1 2 3])]
    (is (= [[2 "n=1"] [4 "n=2"] [6 "n=3"]] (mapv -get xs)))
    (is (= [12 14 16] (mapv #(-get-with % 10) xs)))))

(deftest loop-locals-are-captured-per-iteration
  (let [xs (loop [i 0 acc []]
             (if (= i 3)
               acc
               (recur (inc i) (conj acc (reify Getter
                                          (-get [_] i)
                                          (-get-with [_ x] (+ i x)))))))]
    (is (= [0 1 2] (mapv -get xs)))))

(defn make-shadowed [x]
  (reify Getter
    (-get [_] x)
    (-get-with [_ x] x)))

(deftest a-method-param-shadows-a-captured-local
  (let [g (make-shadowed :outer)]
    (is (= :outer (-get g)))
    (is (= :param (-get-with g :param)))))

(defn make-entry-parser [entries]
  (let [keyset (into {} (map-indexed (fn [i [k]] [k {:order i}]) entries))
        children (mapv first entries)]
    (reify Getter
      (-get [_] keyset)
      (-get-with [_ _] children))))

(deftest a-captured-map-is-not-replaced-by-a-later-instance
  (testing "the shape malli's entry parser has"
    (let [p1 (make-entry-parser [[:x :int]])
          p2 (make-entry-parser [[:y :int]])]
      (is (= {:x {:order 0}} (-get p1)))
      (is (= [:x] (-get-with p1 nil)))
      (is (= {:y {:order 0}} (-get p2))))))

(deftype Box [v]
  Getter
  (-get [_] (reify Getter
              (-get [_] v)
              (-get-with [_ x] [v x])))
  (-get-with [_ _] v))

(deftest a-reify-inside-a-deftype-method-captures-the-field
  (let [r1 (-get (->Box 1))
        r2 (-get (->Box 2))]
    (is (= 1 (-get r1)))
    (is (= 2 (-get r2)))))

(deftest a-field-reads-from-the-instance-when-a-param-reuses-the-this-name
  (is (= 3 (-get-with (->Box 3) :ignored))))

(deftest a-reify-is-equal-only-to-itself
  (let [a (make-getter :same)
        b (make-getter :same)]
    (is (= a a))
    (is (not= a b))
    (is (= 2 (count (hash-set a b))))))
