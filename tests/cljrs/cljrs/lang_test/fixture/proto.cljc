(ns cljrs.lang-test.fixture.proto
  "A protocol in a namespace of its own, for `protocol-impl` to name through an
  alias and through its full name. No tests here.")

(defprotocol IThing
  (-describe [this]))
