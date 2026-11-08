(ns clojure.core-test.descendants
  (:require [clojure.test :refer [are deftest is testing use-fixtures]]
            [clojure.core-test.portability #?(:cljs :refer-macros :default :refer) [when-var-exists] :as p]))

(when-var-exists #?(:rust make-hierarchy :default descendants)

  ; Some types for testing descendants by type
  (defprotocol TestDescendantsProtocol)
  (defrecord TestDescendantsRecord [] TestDescendantsProtocol)
  (deftype TestDescendantsType [] TestDescendantsProtocol)

  (def record-tag #?(:rust ::record-type :default TestDescendantsRecord))
  (def type-tag #?(:rust ::type-type :default TestDescendantsType))
  (def p0-tag #?(:rust ::p-0 :default 'ns/p-0))

  ; A global hierarchy for testing `descendants tag` and `descendants h tag`
  (def global-hierarchy [[record-tag ::record]
                         [::t ::p-1]
                         [::t ::p-2]
                         [::p-1 p0-tag]
                         [::p-2 ::root]
                         [p0-tag ::root]])

  (defn register-global-hierarchy []
    (doseq [[tag parent] global-hierarchy]
      (derive tag parent)))

  (defn unregister-global-hierarchy []
    (doseq [[tag parent] global-hierarchy]
      (underive tag parent)))

  ;; Basilisp does not support clojure.test style fixtures right now
  ;; https://github.com/basilisp-lang/basilisp/issues/1306
  (defn with-global-hierarchy [#?@(:lpy [] :default [tests])]
    (register-global-hierarchy)
    #?(:lpy (yield) :default (tests))
    (unregister-global-hierarchy))

  #?(:rust (register-global-hierarchy)
     :default (use-fixtures :once with-global-hierarchy))

  ; A hierarchy for testing `descendants h tag`
  (def datatypes
    (-> (make-hierarchy)
        (derive record-tag ::datatype)
        (derive type-tag ::datatype)
        (derive type-tag ::mutable)))

  ; Another hierarchy for testing `descendants h tag`
  (def diamond
    (-> (make-hierarchy)
        (derive ::a ::root)
        (derive ::b ::a)
        (derive ::c ::a)
        (derive ::d ::b)
        (derive ::d ::c)))

  (deftest test-descendants

    (testing "descendants tag"

      (testing "returns descendants by relationship globally defined with derive"
        (are [expected tag] (= expected (descendants tag))
                            nil ::t
                            #{::t ::p-1} p0-tag
                            #{::t ::p-1 ::p-2 p0-tag} ::root
                            #{::t} ::p-2
                            #{#?(:bb 'clojure.core_test.descendants/TestDescendantsRecord :default record-tag)} ::record))

      (testing "cannot get descendants by type inheritance"
        #?@(:lpy
            [(is (nil? (descendants TestDescendantsProtocol)))
             (is (p/thrown? (descendants python/object)))]
            :cljs
            [(is (p/thrown? (descendants TestDescendantsProtocol)))
             (is (p/thrown? (descendants js/Object)))]
            :rust
            []
            :default
            [(is (nil? (descendants TestDescendantsProtocol)))
             (is (p/thrown? (descendants Object)))]))

      (testing "does not throw on invalid tag"
        (are [tag] (nil? (descendants tag))
                   nil
                   "anything"
                   42
                   3.14
                   true
                   false
                   []
                   {}
                   #{}
                   '())))

    (testing "descendants h tag"

      (testing "returns only descendants declared in h, whether the tag is in global hierarchy or not"
        (are [expected h tag] (= expected (descendants h tag))

                              ; tag in h and not in global hierarchy
                              nil diamond ::d
                              #{::d} diamond ::b
                              #{::b ::c ::d} diamond ::a
                              #?(:bb      #{'clojure.core_test.descendants/TestDescendantsRecord 'clojure.core_test.descendants/TestDescendantsType}
                                 :default #{record-tag type-tag}) datatypes ::datatype
                              #?(:bb      #{'clojure.core_test.descendants/TestDescendantsType}
                                 :default #{type-tag}) datatypes ::mutable

                              ; tag in both h and global hierarchy, only descendants in h are returned
                              #{::a ::b ::c ::d} diamond ::root

                              ; tag not in h but in global hierarchy
                              nil datatypes ::root
                              nil datatypes ::p-1
                              nil datatypes ::p-2

                              ; tag neither in h nor in global hierarchy
                              nil datatypes ::d
                              nil datatypes ::b
                              nil datatypes ::a))

      #?(:rust "Clojurust has no host-type inheritance"
         :default
         (testing "cannot get descendants by type inheritance, whether the tag is in h or not"
           (are [h] #?(:lpy     (p/thrown? (descendants h python/object))
                       :cljs    (p/thrown? (descendants h js/Object))
                       :default (p/thrown? (descendants h Object)))
                    ; tag in h
                    (derive (make-hierarchy) #?(:lpy python/object :cljs js/Object :default Object) ::object)
                    ; tag not in h
                    diamond
                    datatypes)))

      (testing "does not throw on invalid tag or hierarchy"
        (are [invalid] (nil? (descendants invalid invalid))
                       nil
                       "anything"
                       42
                       3.14
                       true
                       false
                       []
                       {}
                       #{}
                       '())))))
