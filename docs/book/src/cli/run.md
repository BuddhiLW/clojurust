# cljrs run

Interpret a `.cljrs` or `.cljc` source file.

```
cljrs run [OPTIONS] <FILE>
```

All top-level forms in `FILE` are evaluated in order. The return value of the
last form is discarded; side effects (output, file writes, etc.) are the
intended mechanism for a `run` program to produce observable results.

## Arguments

| Argument | Description |
|---|---|
| `<FILE>` | Path to the source file (`.cljrs` or `.cljc`) |

## Options

### `--src-path <DIR>`

Add `DIR` to the list of directories searched when resolving `require`. May be
repeated to add multiple directories.

```
cljrs run --src-path src --src-path lib my-program.cljrs
```

Paths declared in `:paths` of the nearest `cljrs.edn` are appended automatically
after CLI paths.

### `--gc-soft-limit-mb <MB>`

Soft memory limit for the GC in megabytes. When one isolate's heap exceeds
this value, a collection is triggered at the next safepoint. Each isolate heap
applies the limit independently. The value also sets the process-wide memory
governor's soft limit: when managed memory across all isolates reaches it, the
governor asks the allocating isolate (and the largest heap) to collect.

Because one value serves both roles, a program running several isolates can
keep the process at or above the governor's soft limit while every heap stays
under its own trigger: *N* isolates each just under the limit total about *N*
times it. The governor then stays at `Yellow` and keeps requesting
collections (at most once per isolate per collection, and not while a heap is
below its post-collection size). For multi-isolate programs, set the process
budget separately with `CLJRS_MEMORY_SOFT_LIMIT_MB` and
`CLJRS_MEMORY_HARD_LIMIT_MB`, and leave `--gc-soft-limit-mb` unset.

Given alone, it leaves the hard limit at its default (raised to the soft limit
if the soft limit is larger).

### `--gc-hard-limit-mb <MB>`

Process-wide managed-memory hard limit in megabytes. It is **not enforced
yet**: the memory governor runs in observe-only mode, and allocations above the
limit succeed. `--gc-stats` reports how many allocations went over it. Without
`--gc-soft-limit-mb`, the soft limit is 75% of this value.

A zero hard limit, or a soft limit above the hard limit, is rejected.

## Examples

```
# Run a file in the current directory
cljrs run hello.cljrs

# Run with a source path for namespace resolution
cljrs run --src-path src src/myapp/core.cljrs

# Run and write GC stats to stderr on exit
cljrs --gc-stats run my-program.cljrs
```
