(ns clojure.core-test.special-symbol-qmark
  (:require [clojure.test :refer [deftest testing are]]
            [clojure.core-test.portability #?(:cljs :refer-macros :default :refer) [when-var-exists]]))

(when-var-exists special-symbol?
  (deftest test-special-symbol?

    (testing "special symbols"
      (are [arg] (special-symbol? 'arg)
                 ;; Basilisp does not recognize these as special symbols.
                 #?@(:lpy []
                     :phel [&]
                     :rust []
                     :default [&
                               case*
                               new])
                 ;; Phel does not recognize these as special symbols.
                 #?@(:phel []
                     :rust [.
                            fn*
                            let*
                            loop*
                            set!]
                     :default [.
                               deftype*
                               fn*
                               let*
                               letfn*
                               loop*
                               set!])

                 #?@(:rust [] :default [catch])
                 def
                 do
                 #?@(:rust [] :default [finally])
                 if
                 quote
                 recur
                 throw
                 try
                 var))

    (testing "not special symbols"
      (are [arg] (not (special-symbol? arg))
                 'a-symbol
                 'a-ns/a-qualified-symbol
                 #?@(:rust [] :default ['defn])
                 'import
                 "not a symbol"
                 :k
                 0
                 0.0
                 true
                 false
                 nil))))
