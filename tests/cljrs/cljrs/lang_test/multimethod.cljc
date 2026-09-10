(ns cljrs.lang-test.multimethod
  "`defmulti` / `defmethod` dispatch: hierarchy, specificity, preference, the
  method table, and the shape of the `defmulti` head.

  Each test derives in its own keyword namespace: `derive` without an explicit
  hierarchy mutates the global one, and the tests share a runtime."
  (:require [clojure.test :refer [deftest is testing]]))

;; ── Inheritance-driven dispatch ──────────────────────────────────────────────

(derive ::rect ::shape)
(derive ::square ::rect)

(defmulti area-of identity)
(defmethod area-of ::shape [_] :shape)

(deftest dispatch-finds-a-parents-method
  (testing "a value with no exact method reaches its parent's"
    (is (= :shape (area-of ::rect)))))

(deftest dispatch-walks-transitive-ancestors
  (testing "the walk does not stop at the immediate parent"
    (is (= :shape (area-of ::square)))))

(defmulti specific-of identity)
(defmethod specific-of ::shape [_] :shape)
(defmethod specific-of ::rect [_] :rect)

(deftest the-most-specific-method-wins
  (testing "::square inherits from both, and the nearer one is chosen"
    (is (= :rect (specific-of ::square)))))

(defmulti exact-of identity)
(defmethod exact-of ::shape [_] :inherited)
(defmethod exact-of ::square [_] :exact)

(deftest an-exact-method-beats-an-inherited-one
  (testing "an exact match short-circuits the ancestor walk"
    (is (= :exact (exact-of ::square)))))

(defmulti fallback-of identity)
(defmethod fallback-of ::shape [_] :shape)
(defmethod fallback-of :default [_] :fallback)

(deftest default-still-applies-to-unrelated-values
  (testing "a value outside the hierarchy lands on :default"
    (is (= :fallback (fallback-of ::unrelated)))))

;; ── Ambiguity and prefer-method ──────────────────────────────────────────────

(derive ::amphibian ::land)
(derive ::amphibian ::water)

(defmulti travel-of identity)
(defmethod travel-of ::land [_] :land)
(defmethod travel-of ::water [_] :water)

(deftest two-unrelated-parents-are-ambiguous-until-preferred
  (testing "an unresolved tie throws, and prefer-method resolves it"
    (is (thrown? Exception (travel-of ::amphibian)))
    (prefer-method travel-of ::water ::land)
    (is (= :water (travel-of ::amphibian)))))

;; ── Composite dispatch values ────────────────────────────────────────────────

(defmulti collide-of (fn [a b] [a b]))
(defmethod collide-of [::shape ::shape] [_ _] :shapes)

(deftest vector-dispatch-values-match-element-wise
  (testing "each element of the dispatch vector is matched through the hierarchy"
    (is (= :shapes (collide-of ::rect ::square)))))

;; ── The method table is data ─────────────────────────────────────────────────

(defmulti table-of identity)
(defmethod table-of ::rect [_] :rect)
(defmethod table-of ::square [_] :square)

(deftest methods-returns-the-installed-table
  (testing "methods is a map keyed by dispatch value"
    (is (= 2 (count (methods table-of))))
    (is (contains? (methods table-of) ::rect))))

;; Its own table: `remove-method` mutates the var, and test order is not fixed.
(defmulti emptied-of identity)
(defmethod emptied-of ::rect [_] :rect)
(defmethod emptied-of ::square [_] :square)

(deftest remove-method-empties-the-table
  (testing "an emptied table has no methods — count says so, printing does not"
    (remove-method emptied-of ::rect)
    (remove-method emptied-of ::square)
    (is (= 0 (count (methods emptied-of))))))

;; ── Retracting a derivation ──────────────────────────────────────────────────

(derive ::coupe ::car)

(defmulti wheels-of identity)
(defmethod wheels-of ::car [_] :car)

(deftest underive-retracts-the-inherited-match
  (testing "the dispatch cache does not outlive the derivation that filled it"
    (is (= :car (wheels-of ::coupe)))
    (underive ::coupe ::car)
    (is (thrown? Exception (wheels-of ::coupe)))))

(defmulti late-of identity)
(defmethod late-of ::vehicle [_] :vehicle)
(defmethod late-of :default [_] :fallback)

(deftest derive-invalidates-a-cached-default-method
  (testing "a derivation made after the first call is still seen"
    (is (= :fallback (late-of ::truck)))
    (derive ::truck ::vehicle)
    (is (= :vehicle (late-of ::truck)))))

;; ── The defmulti head ────────────────────────────────────────────────────────
;;
;; `(defmulti name docstring? attr-map? dispatch-fn & options)`. The JVM leg is
;; the oracle for the whole head.

(defmulti doc-of "the docstring" identity)

(defmulti attr-of {:doc "the attr-map"} identity)

(defmulti both-of "the docstring" {:doc "the attr-map"} identity)

(defmulti ^{:doc "on the name"} marked-of {:doc "the attr-map"} identity)

(deftest the-defmulti-head-carries-its-documentation
  (testing "a docstring reaches the var"
    (is (= "the docstring" (:doc (meta (var doc-of))))))
  (testing "an attr-map reaches the var"
    (is (= "the attr-map" (:doc (meta (var attr-of))))))
  (testing "a docstring outranks an attr-map :doc"
    (is (= "the docstring" (:doc (meta (var both-of))))))
  (testing "an attr-map outranks metadata written on the name"
    (is (= "the attr-map" (:doc (meta (var marked-of)))))))

(deftest a-defmulti-still-dispatches-with-a-full-head
  (testing "parsing the head did not consume the dispatch fn"
    (defmethod doc-of :x [_] :dispatched)
    (is (= :dispatched (doc-of :x)))))

(deftest a-defmulti-with-no-dispatch-function
  ;; DIVERGENCE: the JVM accepts the form and fails only at the call.
  (testing "cljrs rejects the form, the JVM defers to the call"
    #?(:rust (is (thrown? Exception (eval (read-string "(defmulti no-dispatch)"))))
       :clj (is (var? (eval (read-string "(defmulti no-dispatch)"))))))) 
