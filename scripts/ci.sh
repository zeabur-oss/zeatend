#!/usr/bin/env bash
# One CI job, as the shared workflow of zeabur-oss/.github runs it and as anyone runs it locally:
# the workflow only checks out, installs Nix and restores caches, then calls this script.
#
#   scripts/ci.sh <job>      # run one job; it enters the Nix dev shell itself
#   scripts/ci.sh --list     # the job names
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

jobs=(format clippy docs test cargo-deny)

if [[ "${1:-}" == --list ]]; then
  printf '%s\n' "${jobs[@]}"
  exit 0
fi
job="${1:?usage: scripts/ci.sh <job> | --list}"
printf '%s\n' "${jobs[@]}" | grep -qx -- "$job" ||
  { echo "error: no job named $job (scripts/ci.sh --list)" >&2; exit 1; }

# The marker tells a re-run inside the dev shell apart from a call outside it.
if [[ -z "${ZEABUR_CI_SHELL:-}" ]]; then
  export ZEABUR_CI_SHELL=1
  exec nix develop "$root" --command "$0" "$@"
fi

export RUSTDOCFLAGS="-D warnings"

case "$job" in
  format)
    cargo fmt --all --check
    nixfmt --check flake.nix
    shellcheck scripts/*.sh
    actionlint
    ;;
  clippy) cargo lint ;;
  docs) cargo doc --workspace --no-deps --document-private-items --locked ;;
  test) cargo nextest run --workspace --all-features --locked --no-tests=warn ;;
  cargo-deny) cargo deny --locked check ;;
esac
