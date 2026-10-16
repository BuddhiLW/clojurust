(ns cljrs.lang-test.protocol-head
  "What the *head* of `defprotocol`, `extend-type` and `extend-protocol` means.

  These three are Clojure macros over the `protocol*` and `extend` primitives.
  A macro that replaces a Rust special form is a second implementation of that
  form's contract, and whatever its author did not know about is lost silently:
  no unresolved symbol, no type error, nothing in the diff. So the parts that
  carry no dispatch weight are exactly the parts to pin, because nothing else
  will notice them going missing.

  The JVM leg is the oracle: every assertion here holds on both runtimes."
  (:require [clojure.test :refer [deftest is testing]]))

;; ── defprotocol: the docstrings and arglists that `doc` reports ──────────────

(defprotocol Greet
  "How a thing greets."
  (greet [this] [this loud] "Greet, optionally loudly.")
  (wave [this]))

(deftest the-protocol-docstring-reaches-the-var
  (testing "a leading string is the protocol's :doc, not a dropped form"
    (is (= "How a thing greets." (:doc (meta #'Greet))))))

(deftest a-method-docstring-reaches-its-var
  (testing "the trailing string in a method spec is that method's :doc"
    (is (= "Greet, optionally loudly." (:doc (meta #'greet))))))

(deftest a-method-with-no-docstring-has-none
  (testing "the last form is a parameter vector, not a docstring"
    (is (nil? (:doc (meta #'wave))))))

(deftest every-parameter-vector-becomes-an-arglist
  (testing "not just the first: dispatch reads one, `doc` prints all of them"
    (is (= '([this] [this loud]) (:arglists (meta #'greet))))
    (is (= '([this]) (:arglists (meta #'wave))))))

;; ── extend-type / extend-protocol: multi-arity method bodies ────────────────

(defrecord Person [nom])

(extend-type Person
  Greet
  (greet ([_] :hello) ([_ loud] [:hello loud]))
  (wave [_] :wave))

(deftest extend-type-accepts-grouped-arities
  (testing "(m ([a] …) ([a b] …)) is the only way to give one method several
            arities at a single call site, and it is legal Clojure"
    (let [p (->Person "ana")]
      (is (= :hello (greet p)))
      (is (= [:hello :loudly] (greet p :loudly))))))

(deftest extend-type-still-accepts-the-flat-shape
  (testing "the grouped shape must not cost the ordinary one"
    (is (= :wave (wave (->Person "ana"))))))

(defprotocol Sized (sized [this] [this unit]))

(extend-protocol Sized
  Person
  (sized ([_] :one) ([_ unit] [:one unit])))

(deftest extend-protocol-accepts-grouped-arities-too
  (testing "both surface forms regroup into the same `extend` call"
    (let [p (->Person "ana")]
      (is (= :one (sized p)))
      (is (= [:one :cm] (sized p :cm))))))

(deftest an-extended-type-satisfies-the-protocol
  (testing "the impl table actually received the entry"
    (is (satisfies? Greet (->Person "ana")))
    (is (satisfies? Sized (->Person "ana")))))
