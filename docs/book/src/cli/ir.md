# cljrs ir

Inspect, pre-lower, and visualize clojurust's intermediate representation
(IR). Groups three subcommands:

```
cljrs ir <build|dump|viz> [OPTIONS]
```

| Subcommand | Description |
|---|---|
| [`build`](#cljrs-ir-build) | Pre-lower namespaces to IR and write a serialized bundle |
| [`dump`](#cljrs-ir-dump) | Print a human-readable dump of a serialized IR bundle |
| [`viz`](#cljrs-ir-viz) | Render the optimised IR for a source file to a self-contained HTML page |

---

## `cljrs ir build`

Boots a standard environment, lowers every function in the requested
namespaces to IR, and serializes the result to a bundle file.

```
cljrs ir build [OPTIONS]
```

A bundle produced by `build` is loaded back at startup with the public
`cljrs_eval::load_prebuilt_ir` API, which matches bundle entries to the live
`ir_arity_id`s assigned when the target functions are defined and populates
the IR cache directly — the functions execute at Tier 1 (the IR interpreter)
from their very first call, skipping the warmup that background lowering
normally needs. This is most useful for cutting cold-start latency on targets
that can't run the background lowering worker, such as an embedder built for
`wasm32`.

### Options

#### `-n, --ns <NS>`

Namespace to lower. May be repeated. Defaults to `clojure.core` if omitted.
Non-`clojure.core` namespaces are `require`d from `--src-path` before
lowering.

#### `-o, --output <PATH>`

Output file path for the serialized IR bundle. Defaults to `ir_bundle.bin`.

#### `--src-path <DIR>`

Add `DIR` to the source path used to resolve `--ns` namespaces other than
`clojure.core`. May be repeated.

#### `-v, --verbose`

Print per-arity lowering progress to stderr.

### Example

```
cljrs ir build --ns clojure.core -o core.ir.bin
cljrs ir build --ns my.app.core --src-path src -o app.ir.bin -v
```

---

## `cljrs ir dump`

Print a human-readable dump of every function in a serialized IR bundle.

```
cljrs ir dump <INPUT>
```

### Arguments

| Argument | Description |
|---|---|
| `<INPUT>` | Path to a bundle written by `cljrs ir build` |

### Example

```
cljrs ir dump app.ir.bin
```

---

## `cljrs ir viz`

Render the optimised IR for a source file to a self-contained HTML page.

```
cljrs ir viz [OPTIONS] <FILE>
```

The HTML output shows the source side-by-side with the IR, with regions
colour-coded by the bump-allocation optimiser's results. Allocations that did
not make it into a region are annotated with their escape verdict and the
blamed use site.

This subcommand is primarily a debugging aid for the IR optimisation pipeline.

### Arguments

| Argument | Description |
|---|---|
| `<FILE>` | Source file to lower to IR |

### Options

#### `-o, --out <FILE>`

Output path for the HTML file. If omitted, the output is written alongside
the source file with an `.ir.html` extension:

```
src/myapp/core.cljrs  →  src/myapp/core.cljrs.ir.html
```

#### `--src-path <DIR>`

Add `DIR` to the source path for `require` resolution. May be repeated.

#### `--quiet`

Suppress the `[ir viz] wrote …` progress line on stderr.

### Example

```
cljrs ir viz src/myapp/core.cljrs
# writes: src/myapp/core.cljrs.ir.html

cljrs ir viz src/myapp/core.cljrs --out /tmp/core.html --quiet
```

Open the resulting HTML file in a browser to explore the IR.

### Interpreting the output

- **Green regions** — allocations placed in a bump-allocation region; they do
  not incur GC heap pressure.
- **Red / yellow annotations** — allocations that escaped the region, labelled
  with the reason (returned, captured by closure, stored in heap object, etc.).
- Clicking a source line highlights the corresponding IR instructions and vice
  versa.
