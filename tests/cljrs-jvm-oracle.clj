;; The JVM half of the differential oracle over `tests/cljrs`.
;;
;; `tests/cljrs` is written in `.cljc` on purpose: the same corpus is the
;; cljrs regression suite AND a statement about what Clojure means. Running it
;; on real Clojure is what keeps the second claim honest — a test that passes
;; here and on `cljrs test --src-path ./tests/cljrs` pins agreed behaviour,
;; and one that passes on only one side is a divergence that has to be argued
;; for in a reader conditional rather than discovered years later.
;;
;;     clojure -Sdeps '{:paths ["tests/cljrs"]}' -M tests/cljrs-jvm-oracle.clj
;;
;; Namespaces are derived from the tree rather than listed, so this cannot
;; drift out of step with `cljrs`'s own discovery (which walks the same
;; directory for the same extensions).

(require '[clojure.java.io :as io]
         '[clojure.string :as str]
         'clojure.test)

(def ^:private root (io/file "tests/cljrs"))

(defn- namespace-of
  "Mirror `file_to_namespace`: path under the root, minus the extension, with
  separators as dots and underscores as hyphens."
  [^java.io.File f]
  (-> (.getPath f)
      (str/replace-first (str (.getPath root) "/") "")
      (str/replace #"\.cljc$" "")
      (str/replace "/" ".")
      (str/replace "_" "-")
      symbol))

(let [namespaces (->> (file-seq root)
                      (filter #(.isFile ^java.io.File %))
                      (filter #(str/ends-with? (.getName ^java.io.File %) ".cljc"))
                      (map namespace-of)
                      sort)]
  (when (empty? namespaces)
    (println "no .cljc namespaces found under" (.getPath root))
    (System/exit 1))

  (println "JVM oracle over" (count namespaces) "namespace(s):")
  (doseq [n namespaces] (println "  " n))
  (println)

  (doseq [n namespaces] (require n))

  (let [{:keys [fail error]} (apply clojure.test/run-tests namespaces)]
    (shutdown-agents)
    (System/exit (if (pos? (+ fail error)) 1 0))))
