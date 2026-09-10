(ns cljrs.lang-test.fixture.proto
  "A protocol that lives somewhere other than the namespace implementing it.

  Carries no tests of its own — it exists so `protocol-impl` can name
  `IThing` through an alias and through its fully qualified name, which is the
  shape every port/adapter design takes and the one that used to fail.")

(defprotocol IThing
  (-describe [this]))
