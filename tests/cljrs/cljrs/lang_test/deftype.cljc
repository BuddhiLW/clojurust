(ns cljrs.lang-test.deftype
  "`deftype` as a *language* surface: the positional constructor, field access,
  protocol method bodies, and mutable fields.

  The host half of the same feature stays in Rust
  (`crates/cljrs-runtime/tests/deftype_types.rs`): an assertion written here
  cannot see which `EvalError` variant came back, and
  `deftype_mutable_tiered.rs` pins the same scripts with IR lowering forced on,
  which is a tier property, not a language one."
  (:require [clojure.test :refer [deftest is testing]]))

;; ── The type itself ──────────────────────────────────────────────────────────

(deftype Point [x y])

(deftest positional-constructor
  (testing "->T binds the fields in declaration order"
    (let [p (->Point 3 4)]
      (is (= 3 (.-x p)))
      (is (= 4 (.-y p))))))

(deftest type-name-is-interned
  (testing "the type symbol resolves, so instance? can name it"
    (is (instance? Point (->Point 1 2)))))

(deftype Marked ^:marker [x])

(deftest field-vector-may-carry-metadata
  (testing "a marker on the field vector is structurally transparent"
    (is (= 7 (.-x (->Marked 7))))))

;; ── Protocol method bodies ───────────────────────────────────────────────────

(defprotocol Described
  (describe [this]))

(defprotocol Scaled
  (scale-10 [this])
  (scale-100 [this]))

(deftype Pair [a b]
  Described
  (describe [_] [a b]))

(deftest immutable-fields-are-in-scope-in-a-method-body
  (testing "a method body sees the fields as plain locals"
    (is (= [1 2] (describe (->Pair 1 2))))))

(deftype Multi [a]
  Described
  (describe [_] [:multi a])
  Scaled
  (scale-10 [_] (* a 10))
  (scale-100 [_] (* a 100)))

(deftest one-type-can-implement-several-protocols
  (testing "every protocol in the tail is installed"
    (let [t (->Multi 2)]
      (is (= [:multi 2] (describe t)))
      (is (= 20 (scale-10 t)))
      (is (= 200 (scale-100 t))))))

;; ── Mutable fields ───────────────────────────────────────────────────────────

(defprotocol Counter
  (bump [this])
  (bump-twice [this])
  (peek-n [this]))

(deftype Unsync [^:unsynchronized-mutable n]
  Counter
  (bump [_] (set! n (inc n)))
  (bump-twice [_] (set! n (inc n)) (set! n (inc n)) n)
  (peek-n [_] n))

(deftest set-bang-on-a-bare-field-name
  (testing "a write through the bare field name survives the call"
    (let [c (->Unsync 0)]
      (bump c)
      (bump c)
      (is (= 2 (peek-n c))))))

(deftest a-write-is-visible-to-a-later-read-in-the-same-method
  (testing "two writes in one body compose"
    (is (= 7 (bump-twice (->Unsync 5))))))

(deftest every-write-in-a-hot-method-accumulates
  (testing "500 calls to a mutating method land 500 writes"
    (let [c (->Unsync 0)]
      (dotimes [_ 500] (bump c))
      (is (= 500 (peek-n c))))))

(deftype Volatile [^:volatile-mutable n]
  Counter
  (bump [_] (set! n (inc n)))
  (bump-twice [_] (set! n (inc n)) (set! n (inc n)) n)
  (peek-n [_] n))

(deftest volatile-mutable-behaves-like-unsynchronized-mutable
  (testing "the two mutability hints are interchangeable at this level"
    (let [c (->Volatile 41)]
      (bump c)
      (is (= 42 (peek-n c))))))

(deftype Box [^:unsynchronized-mutable v])

(deftest set-bang-on-an-explicit-field-target
  ;; DIVERGENCE. On the JVM a `^:unsynchronized-mutable` field is private to
  ;; the methods of its own type: `(set! (.-v inst) v)` from outside raises
  ;; `IllegalArgumentException: No matching field found`. cljrs allows the
  ;; external write. Recorded in both directions rather than asserted in one,
  ;; so the day cljrs tightens this the `:rust` branch fails and names it.
  (testing "(set! (.-field inst) v) from outside a method body"
    (let [b (->Box :old)]
      #?(:rust (do (set! (.-v b) :new)
                   (is (= :new (.-v b))))
         :clj (is (thrown? IllegalArgumentException (set! (.-v b) :new)))))))

(deftest mutating-one-instance-does-not-touch-another
  ;; Same divergence as above: only cljrs can perform the external write, so
  ;; only cljrs can ask whether it stayed on one instance. On the JVM the
  ;; per-instance question is answered through a method instead.
  (testing "the mutable cell is per-instance, not per-type"
    #?(:rust (let [a (->Box 1)
                   b (->Box 1)]
               (set! (.-v a) 99)
               (is (= 99 (.-v a)))
               (is (= 1 (.-v b))))
       :clj (let [a (->Unsync 0)
                  b (->Unsync 0)]
              (bump a)
              (is (= 1 (peek-n a)))
              (is (= 0 (peek-n b)))))))

(defprotocol Reporter
  (report [this]))

(deftype Labelled [label ^:unsynchronized-mutable n]
  Reporter
  (report [_] (set! n (inc n)) [label n]))

(deftest immutable-and-mutable-fields-coexist
  (testing "one field vector may mix the two kinds"
    (let [t (->Labelled "hits" 0)]
      (report t)
      (is (= ["hits" 2] (report t))))))

;; ── set! still means what it meant ───────────────────────────────────────────

(def ^:dynamic *v* 1)

(deftest set-bang-on-a-dynamic-var-is-unaffected
  (testing "the field-target form did not capture plain var set!"
    (is (= 3 (binding [*v* 2] (set! *v* 3) *v*)))))
