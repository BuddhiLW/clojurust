#!/usr/bin/env bash
#
# Decide whether tonight's nightly should produce a release, and pick its
# version number.
#
# A release happens only when the tree has changed in a way that affects the
# published crates since the previous release.  Documentation and website edits
# alone are not worth a version bump.
#
# Writes to $GITHUB_OUTPUT (or stdout when run locally):
#   should_release  true | false
#   version         the version to publish, e.g. 0.1.253
#   tag             v<version>
#   prev_tag        the release this one follows, empty on the first release
#   prev_base       commit that release was cut from, used as the diff base
#   head_sha        the commit this decision was made about
#   changed_files   newline-joined list of release-relevant paths (may be empty)
#
# Environment:
#   BASE_VERSION  major.minor prefix for the version series (default "0.1")
#   FORCE         "true" to release regardless of which paths changed
#   GH_TOKEN      optional; lets the script detect an unfinished release
#
set -euo pipefail

BASE_VERSION="${BASE_VERSION:-0.1}"
FORCE="${FORCE:-false}"

# Paths that never justify a release on their own.  git pathspecs, matched
# against the full path from the repository root.
IGNORED_PATHS=(
  ':(exclude,glob)docs/**'
  ':(exclude,glob)www/**'
  ':(exclude,glob)**/*.md'
  ':(exclude)epl-v10.html'
)

emit() {
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    printf '%s\n' "$1" >>"$GITHUB_OUTPUT"
  else
    printf '%s\n' "$1"
  fi
}

# A release tag points at a `chore: release X` commit that bumps every
# Cargo.toml.  That commit is never merged back into the branch, so the merge
# base — not the tag itself — is the branch commit the release was cut from,
# and the only honest comparison point.  Diffing the tag directly would report
# every manifest as changed on every run.
base_of() { git merge-base "$1" HEAD; }

# A GitHub release is the marker that a version finished shipping: the workflow
# creates it after the crates are on crates.io.  When we cannot check (no gh,
# no token), assume finished — that keeps the script from re-publishing on a
# hunch.
release_finished() {
  command -v gh >/dev/null 2>&1 || return 0
  [[ -n "${GH_TOKEN:-${GITHUB_TOKEN:-}}" ]] || return 0
  gh release view "$1" >/dev/null 2>&1
}

# Existing patch numbers in the series, ascending.  Tags are the source of
# truth for the version series: every published release pushes one, and unlike
# a workflow run number they survive the workflow file being replaced.
mapfile -t patches < <(git tag --list "v${BASE_VERSION}.*" \
  | sed -n "s|^v${BASE_VERSION}\.\([0-9][0-9]*\)$|\1|p" \
  | sort -n)

count=${#patches[@]}
latest_tag=""
prior_tag=""
[[ $count -ge 1 ]] && latest_tag="v${BASE_VERSION}.${patches[count - 1]}"
[[ $count -ge 2 ]] && prior_tag="v${BASE_VERSION}.${patches[count - 2]}"

head_sha=$(git rev-parse HEAD)
resuming=false

if [[ -n "$latest_tag" && "$(base_of "$latest_tag")" == "$head_sha" ]] \
   && ! release_finished "$latest_tag"; then
  # The newest tag was cut from this very commit but never got a GitHub
  # release, so a previous attempt died partway through publishing.  Finish
  # that version instead of allocating a new one: the crates already on
  # crates.io under it would otherwise be stranded, leaving a version where
  # some workspace crates exist and others never will.
  resuming=true
  tag="$latest_tag"
  version="${tag#v}"
  prev_tag="$prior_tag"
  echo "Tag $tag exists at HEAD with no GitHub release — resuming that release."
else
  next_patch=0
  [[ $count -ge 1 ]] && next_patch=$((patches[count - 1] + 1))
  version="${BASE_VERSION}.${next_patch}"
  tag="v${version}"
  prev_tag="$latest_tag"
fi

echo "Previous release tag: ${prev_tag:-<none>}"
echo "Candidate version:    $version"

if [[ -n "$prev_tag" ]]; then
  prev_base=$(base_of "$prev_tag")
  echo "Previous release base: $prev_base"
  changed=$(git diff --name-only "${prev_base}..HEAD" -- . "${IGNORED_PATHS[@]}")
  total=$(git diff --name-only "${prev_base}..HEAD" | wc -l | tr -d ' ')
  relevant=$(printf '%s' "$changed" | grep -c . || true)
  echo "Changed since ${prev_tag}: ${total} file(s), ${relevant} release-relevant."
else
  prev_base=""
  changed=""
fi

if [[ "$resuming" == "true" ]]; then
  should_release=true
elif [[ -z "$prev_tag" ]]; then
  echo "No previous release tag — releasing the initial version."
  should_release=true
elif [[ -n "$changed" ]]; then
  should_release=true
elif [[ "$FORCE" == "true" ]]; then
  echo "Only documentation/website changes, but FORCE is set — releasing anyway."
  should_release=true
else
  should_release=false
fi

if [[ "$should_release" == "true" ]]; then
  echo "Decision: release $tag"
else
  echo "Decision: skip — no non-doc, non-www changes since ${prev_tag}."
fi

emit "should_release=$should_release"
emit "version=$version"
emit "tag=$tag"
emit "prev_tag=$prev_tag"
emit "prev_base=$prev_base"
emit "head_sha=$head_sha"

# Multi-line output needs a heredoc-style delimiter.
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    echo "changed_files<<__CHANGED_EOF__"
    printf '%s\n' "$changed"
    echo "__CHANGED_EOF__"
  } >>"$GITHUB_OUTPUT"
fi

if [[ -n "$changed" ]]; then
  echo "--- release-relevant changes ---"
  printf '%s\n' "$changed" | head -n 50
fi
