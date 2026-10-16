(ns cljrs.lang-test.queue
  "A PersistentQueue is an ordinary seqable collection, and
  `clojure.lang.PersistentQueue/EMPTY` is how source names an empty one.

  The spelling matters for a shared `.cljc`: cljrs has a `(queue)` builtin and
  the JVM does not, so the only way to write an empty queue that both sides
  read is the static member."
  (:require [clojure.test :refer [deftest is testing]]))

(def ^:private E clojure.lang.PersistentQueue/EMPTY)

(defn- q12
  "A two-element queue, built the way source builds one."
  []
  (conj (conj E 1) 2))

(deftest the-empty-queue-resolves-and-is-empty
  (is (zero? (count E)))
  (is (nil? (seq E)))
  (is (nil? (peek E))))

(deftest a-queue-is-seqable
  (testing "seq itself"
    (is (= '(1 2) (seq (q12)))))
  (testing "and every walker built on it"
    (is (= [1 2] (vec (q12))))
    (is (= [1 2] (into [] (q12))))
    (is (= '(2 3) (map inc (q12))))
    (is (= 3 (reduce + 0 (q12))))
    (is (= 2 (last (q12))))))

(deftest a-queue-reports-itself-as-seqable
  (is (seqable? E))
  (is (seqable? (q12))))

(deftest first-and-rest-walk-a-queue-in-fifo-order
  (is (= 1 (first (q12))))
  (is (= '(2) (rest (q12))))
  (is (= '(2) (next (q12))))
  (testing "rest of a one-element queue is empty, not nil"
    (is (= '() (rest (conj E 1))))
    (is (nil? (next (conj E 1))))))

(deftest peek-and-pop-take-from-the-front
  (is (= 1 (peek (q12))))
  (is (= '(2) (seq (pop (q12))))))

(deftest nth-reaches-a-queue-by-walking-it
  ;; A queue is Sequential but not Indexed, so nth has to walk.
  (is (= 1 (nth (q12) 0)))
  (is (= 2 (nth (q12) 1)))
  (is (= :none (nth (q12) 5 :none)))
  (is (thrown? Exception (nth (q12) 5))))

(deftest nth-still-refuses-an-unordered-collection
  ;; The counterpart to the test above: being seqable is not enough for nth,
  ;; and widening the queue case must not widen these.
  (is (thrown? Exception (nth {:a 1} 0)))
  (is (thrown? Exception (nth #{1 2} 0))))

(deftest a-non-seqable-value-is-refused-rather-than-read-as-empty
  ;; The regression this file exists for: these used to answer [], [], nil and
  ;; the init value for a queue, because the shared walker ended quietly on a
  ;; type it did not know instead of raising.
  (is (not (seqable? 5)))
  (is (thrown? Exception (vec 5)))
  (is (thrown? Exception (doall (map inc 5)))))
