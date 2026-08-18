# Release process

Releases are **nightly**, not per-merge. A scheduled workflow looks at what
landed on `main` since the last release and, if any of it affects the published
crates, cuts a new version.

## What runs when

| Workflow | Trigger | Does |
|---|---|---|
| `.github/workflows/ci.yml` | pull requests, pushes to `main` | fmt, check, clippy, tests, Clojure test suite, WASM build |
| `.github/workflows/nightly.yml` | 07:00 UTC daily, or `workflow_dispatch` | decides whether to release, re-runs CI, tags, publishes to crates.io, creates the GitHub release |

## The release gate

`.github/scripts/should-release.sh` decides. It diffs the working tree against
the commit the previous release was cut from and ignores paths that cannot
affect the crates:

- `docs/**`
- `www/**`
- any `*.md`
- `epl-v10.html`

If nothing else changed, the night is a no-op: no tag, no version, no publish.
A day of documentation or website work therefore does not burn a version
number. To release anyway, run the workflow manually with **force** checked.

The diff base is `git merge-base <previous tag> HEAD`, not the tag itself. Each
release tag points at a `chore: release X` commit that bumps every
`Cargo.toml`; that commit is never merged back into `main`, so diffing against
the tag directly would report every manifest as changed on every run and the
gate would never close.

## Versioning

Versions are `0.2.N`, where `N` is one past the highest existing `v0.2.*` tag,
starting from `0.2.0`. The tags are the source of truth for the series — a
workflow run number is not, because it resets whenever the workflow file is
replaced.

The series is set by `BASE_VERSION` in `nightly.yml`. Moving it restarts the
patch counter at 0, so the first release of a new series is `<base>.0`. The
release it *follows* is still the newest tag overall, whatever series that
belongs to: the first `0.2.0` reports its changes against the last `0.1.x` and
is gated on the same path rules, rather than firing unconditionally with a
changelog running back to the first commit.

One exception: if the highest tag was cut from the current `HEAD` and has no
GitHub release, a previous attempt died partway through publishing. The gate
then *resumes* that version rather than allocating a new one, so the crates
already pushed under it are not stranded at a version some of the workspace
will never reach. The GitHub release is what marks a version as finished; it is
created last, after the crates are on crates.io.

`main`'s manifests stay at their development version. The version bump is
committed locally in the release job and reachable only through the tag, so
nothing is pushed to `main`.

## Release notes

`.github/scripts/generate-changelog.sh` assembles the body from two sources:

1. GitHub's own release-notes generator (`POST /releases/generate-notes`),
   which resolves commits to the pull requests that introduced them and credits
   their authors.
2. The raw commit subjects between the two releases, as a fallback and as extra
   material for the summarizer.

If the `ANTHROPIC_API_KEY` secret is set, both are passed to
`.github/scripts/summarize-changelog.sh`, which asks Claude for a short
**Highlights** section prepended to the generated list. This is best-effort: a
missing key, an API error, or a refusal logs a warning and the release goes out
with the mechanically generated notes alone. The generated list below the
highlights is always the authoritative record.

The commit log is passed to the model as untrusted data and the system prompt
says so — a commit message cannot redirect the summarizer.

## Configuration

Repository **secrets**:

| Secret | Required | Used for |
|---|---|---|
| `CARGO_REGISTRY_TOKEN` | yes | publishing to crates.io |
| `ANTHROPIC_API_KEY` | no | the Claude-written highlights section |
| `GITHUB_TOKEN` | automatic | pushing the tag, creating the release |

Repository **variables**:

| Variable | Default | Used for |
|---|---|---|
| `CHANGELOG_MODEL` | `claude-opus-5` | model id for the highlights section |

## Manual runs

`workflow_dispatch` takes two inputs:

- **force** — release even if only docs or website files changed.
- **dry_run** — run the gate, the build, and the note generation, but skip the
  tag push, the crates.io publish, and the GitHub release. The notes land in
  the run summary so you can read what would have shipped.

## Retrying a failed release

Every mutating step is idempotent, so re-running the workflow after a partial
failure is safe:

- the version is recomputed as the unfinished one rather than the next one
  (see [Versioning](#versioning));
- the tag is pushed only if it is not already on the remote;
- crates already live at the target version are excluded from
  `cargo publish --workspace`;
- an existing GitHub release for the tag has its notes updated rather than
  being recreated.
