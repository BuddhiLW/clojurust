(ns cljrs.lang-test.rust-io
  "The file-system questions a library asks before it writes: does the path
  exist, is it a directory, is it a regular file; and the two verbs around
  them, `make-parents` and `delete-file`.

  cljrs answers from `clojure.rust.io`; the JVM answers from `java.io.File`
  through `clojure.java.io`. The helpers below are the whole divergence, so
  every assertion is the same on both legs."
  (:require [clojure.test :refer [deftest is testing]]))

#?(:rust (require '[clojure.rust.io :as rio])
   :clj (require '[clojure.java.io :as jio]))

(defn- exists? [p]
  #?(:rust (rio/exists? p) :clj (.exists (jio/file p))))

(defn- directory? [p]
  #?(:rust (rio/directory? p) :clj (.isDirectory (jio/file p))))

(defn- regular-file? [p]
  #?(:rust (rio/regular-file? p) :clj (.isFile (jio/file p))))

(defn- make-parents [p]
  #?(:rust (rio/make-parents p) :clj (jio/make-parents p)))

(defn- delete-file [p]
  #?(:rust (rio/delete-file p) :clj (jio/delete-file p)))

(defn- tmp-dir []
  #?(:rust (or (System/getenv "TMPDIR") "/tmp")
     :clj (System/getProperty "java.io.tmpdir")))

(defn- fresh-root
  "A directory name nothing else is using, under the platform temp dir."
  []
  (str (tmp-dir) "/" (name (gensym "cljrs-rust-io-test-"))))

(deftest a-missing-path-answers-false-to-every-question
  (let [p (str (fresh-root) "/nope.txt")]
    (is (not (exists? p)))
    (is (not (directory? p)))
    (is (not (regular-file? p)))))

(deftest make-parents-then-spit-then-the-predicates-agree
  (let [root (fresh-root)
        dir (str root "/a/b")
        file (str dir "/c.txt")]
    (testing "the parents come into being, the leaf does not"
      (make-parents file)
      (is (directory? dir))
      (is (exists? dir))
      (is (not (regular-file? dir)))
      (is (not (exists? file))))
    (testing "a written file is a regular file and not a directory"
      (spit file "x")
      (is (exists? file))
      (is (regular-file? file))
      (is (not (directory? file)))
      (is (= "x" (slurp file))))
    (testing "delete-file takes the file, then each now-empty directory"
      (is (delete-file file))
      (is (not (exists? file)))
      (is (directory? dir))
      (is (delete-file dir))
      (is (delete-file (str root "/a")))
      (is (delete-file root))
      (is (not (exists? root))))))

(deftest delete-file-refuses-a-non-empty-directory
  (let [root (fresh-root)
        file (str root "/keep.txt")]
    (make-parents file)
    (spit file "x")
    (is (thrown? Exception (delete-file root)))
    (is (regular-file? file))
    (delete-file file)
    (delete-file root)
    (is (not (exists? root)))))

(deftest delete-file-can-be-asked-to-stay-quiet
  ;; The return value under `silently` differs (the JVM hands back the flag,
  ;; cljrs answers false); what both promise is that nothing is thrown.
  (let [p (str (fresh-root) "/absent.txt")]
    (is (thrown? Exception (delete-file p)))
    (is (nil? (try #?(:rust (rio/delete-file p true)
                      :clj (jio/delete-file p true))
                   nil
                   (catch Exception e e))))))
