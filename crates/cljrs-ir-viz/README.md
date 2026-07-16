# cljrs-ir-viz

HTML visualizer for the clojurust IR.  Generates a single self-contained
HTML file (inline CSS + JS, no external deps) that shows a function's IR
alongside its source, color-coded by the region-allocation optimizer's
output.

**Purpose:** debug the bump-allocation optimizer.  When a value escapes
or otherwise misses region promotion, the visualizer flags it with the
escape-analysis verdict and the use that "blamed" it — making it
obvious why the optimizer left it on the GC heap.

**Status:** Phase-2 tooling crate.  Implemented and tested against
hand-written snippets; not yet integrated with the AOT compiler's
`--emit-ir-html` flag (the CLI exposes it as the `cljrs ir viz`
subcommand instead).

---

## File layout

```
src/
  lib.rs    — public entry point: `render_html` and `RenderOptions`
  render.rs — top-level HTML assembly, function/block/inst rendering,
              source-pane rendering, region color assignment
  region.rs — collect `RegionStart`/`RegionEnd` pairs, compute the set
              of `(block, inst_index)` positions covered by each region
  blame.rs  — pick a representative "blame" use for a non-promoted
              allocation; format use-kind labels and escape-state badges
tests/
  smoke.rs  — lower a small snippet, render to HTML, and assert the
              output is well-formed and contains expected markers
examples/
  dump.rs   — `cargo run -p cljrs-ir-viz --example dump > /tmp/ir.html`
              renders a hand-written demo to stdout
```

---

## Usage

### CLI

```sh
cljrs ir viz path/to/file.cljrs        # writes path/to/file.cljrs.ir.html
cljrs ir viz path/to/file.cljrs -o out.html
cljrs ir viz path/to/file.cljrs --src-path src/    # for require resolution
```

### Library

```rust
use cljrs_ir::lower::{lower_fn_body, optimize};
use cljrs_ir_viz::{render_html, RenderOptions};

let ir = optimize(lower_fn_body(Some("f"), "user", &[], &forms)?);
let html = render_html(&ir, Some(source_text), &RenderOptions::default());
std::fs::write("ir.html", html)?;
```

---

## Public API

```rust
pub fn render_html(
    ir: &cljrs_ir::IrFunction,
    source: Option<&str>,
    opts: &RenderOptions,
) -> String;

pub struct RenderOptions {
    pub title: Option<String>,
}
```

`render_html` walks `ir` plus all subfunctions, runs escape analysis with
an inter-procedural context, and produces a complete HTML document.  The
return value is a self-contained string suitable for writing to disk and
opening in any browser.

---

## What the visualizer shows

For each function:

* **Header** — function name (with parent path for subfunctions),
  parameter list, and source span when known.
* **Allocation summary** — count of region-allocated, heap, and closure
  allocations.
* **Per-block IR** — every instruction with its index, with kinds
  color-coded:
  * `alloc` (heap) — orange
  * `ralloc` (region) — green, with strong tint matching the region's
    color
  * `rstart` / `rend` — italic gray
  * `call`, `store`, `loc`, etc.
* **Region coloring** — every `RegionStart`/`RegionEnd` pair gets a
  deterministic hue (golden-angle spacing).  Instructions inside the
  region get a pale tint of that hue; the actual `RegionAlloc` /
  `RegionStart` / `RegionEnd` markers get a stronger tint plus an accent
  border.  Source lines that produced any of the region's
  `RegionAlloc`s get the same accent border in the gutter.
* **Escape badges** — every `Alloc*` instruction (i.e. one that did
  *not* get promoted) shows its escape verdict (`no-escape`,
  `arg-escape`, `returns`, `escapes`) and the blamed use (e.g. *"return
  value"*, *"stored into heap object in bb1"*, *"arg 0 of known call
  Map"*).  Pure `no-escape` allocations are unusual after optimization
  and indicate a missed promotion opportunity.
* **Hover linking** — hovering an IR instruction highlights its source
  line; hovering a source line highlights all IR insts derived from it.
  Lookup is by line number via `data-line` attributes.

---

## Notes on source mapping

ANF lowering emits `Inst::SourceLoc(span)` markers at the head of each
form's lowering, deduped per `(file, line)` within a block.  These are
pure no-op instructions (`Effect::Pure`, no `dst`) so all existing
analysis and code-generation passes ignore them — they exist only for
this visualizer and other downstream tooling.

The `IrFunction.span` field is currently populated only for
hand-constructed IR; the ANF lowering path does not yet set it for
top-level functions.  Subfunction headers therefore show only their
first `SourceLoc` marker rather than a span range.

---

## Dependencies

| Crate         | Role                                          |
|---------------|-----------------------------------------------|
| `cljrs-ir`    | `IrFunction`, `EscapeContext`, `analyze`, ... |
| `cljrs-types` | `Span` for source location handling           |
| `cljrs-reader` (dev) | parsing source for tests                |
