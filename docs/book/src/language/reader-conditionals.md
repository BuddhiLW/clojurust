# Reader conditionals

Reader conditionals allow a single source file to contain code for multiple
Clojure platforms. clojurust evaluates the `:cljrs` branch.

`:cljrs` is the canonical platform key. The older key `:rust` is still
accepted as an alias and selects the same branch, but new code should use
`:cljrs`.

## Syntax

### Non-splicing form

```clojure
#?(:cljrs  expr-cljrs
   :clj    expr-jvm
   :cljs   expr-clojurescript
   :default expr-fallback)
```

Exactly one branch is selected based on the current platform. Clauses are
tried left to right and the first one whose key is `:cljrs` (or its alias
`:rust`) or `:default` wins, as in Clojure. If no clause matches, the entire
form is skipped (reads as nothing).

The selected branch is a single expression; the whole `#?(...)` form evaluates
to that expression.

```clojure
(def platform #?(:cljrs  "clojurust"
                 :clj    "JVM Clojure"
                 :cljs   "ClojureScript"
                 :default "unknown"))
```

### Splicing form

```clojure
#?@(:cljrs  [a b c]
    :clj    [x y z]
    :default [])
```

The splicing form `#?@(...)` selects a **vector** from the active platform and
splices its elements into the surrounding form. It is only valid inside a list,
vector, map, or set literal.

```clojure
;; Adds platform-specific items to a vector
(def features [#?@(:cljrs  [:gc :cranelift]
                   :clj    [:jvm :hotspot]
                   :default [])])
; => [:gc :cranelift]  (on clojurust)
```

```clojure
;; Platform-specific require in an ns form
(ns myapp.core
  (:require [clojure.string :as str]
            #?@(:cljrs [[:clojurust.system :as sys]]
                :clj   [[:java.lang.System :as sys]])))
```

## File-extension behaviour

| Extension | Platform dispatch |
|---|---|
| `.cljrs` | Always `:cljrs`. Reader conditionals are still supported but `:cljrs` is always the active platform. |
| `.cljc` | Cross-platform. Reader conditional branches are stored as-is; the evaluator selects `:cljrs`. |

## Notes

- The **reader** stores all branches of a `#?(...)` form; only the evaluator
  discards non-matching branches. This means reader-conditional forms can be
  inspected programmatically without losing the other branches.
- Order within a reader conditional matters: keys are checked left-to-right.
  `:default` should come last.
- `:cljr` is ClojureCLR's key, not clojurust's; the clojurust key is `:cljrs`.
- `:rust` is a legacy alias for `:cljrs`. Both keys match, so a form that
  lists both selects whichever comes first. Prefer `:cljrs` in new code.

## Libraries written for `:clj` and `:cljs` only

clojurust never falls back to the `:clj` branch. A `.cljc` library whose forms
have only `:clj` and `:cljs` branches reads those forms as nothing, so it
fails to load. A `deftype` whose field vector is such a form reports it:
`the field list must be a vector ... add a :cljrs branch`.

Such a library needs `:cljrs` branches. [malli](https://github.com/metosin/malli)
is the common case: stock malli releases do not load. A fork with the
branches is under review upstream (metosin/malli#1324). Until it merges, pin
the fork in `cljrs.edn` and fetch it:

```clojure
{:paths ["src"]
 :deps  {metosin/malli {:git/url "https://github.com/BuddhiLW/malli"
                        :git/sha "e7b910ed244d0e294a70766b455d72660dbc75af"}}}
```

```bash
cljrs deps fetch
cljrs run src/my/app.cljrs   # (require '[malli.core :as m]) now loads
```
