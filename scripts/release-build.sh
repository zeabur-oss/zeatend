#!/usr/bin/env bash
# Builds the release archive of this machine's architecture. It publishes nothing.
#
#   scripts/release-build.sh <vX.Y.Z> <amd64|arm64> <dist>
set -euo pipefail

if [[ $# != 3 ]]; then
  echo "usage: release-build.sh <vVERSION> <amd64|arm64> <output-dir>" >&2
  exit 2
fi
version="$1"
arch="$2"
root="$(cd "$(dirname "$0")/.." && pwd)"
: "${IN_NIX_SHELL:?run inside nix develop}"
[[ "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] ||
  { echo "$version is not vX.Y.Z or vX.Y.Z-<pre-release>" >&2; exit 2; }
# mkShell sets SOURCE_DATE_EPOCH to 1980; the commit's time replaces it.
SOURCE_DATE_EPOCH="$(git -C "$root" show -s --format=%ct HEAD)"
export SOURCE_DATE_EPOCH
case "$(uname -s):$arch:$(uname -m)" in
  Linux:amd64:x86_64) target=x86_64-unknown-linux-musl ;;
  Linux:arm64:aarch64) target=aarch64-unknown-linux-musl ;;
  *) echo "cannot build $arch here: each build is native, on Linux" >&2; exit 2 ;;
esac

mkdir -p "$3"
output="$(cd "$3" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
target_dir="${CARGO_TARGET_DIR:-$root/target}"

# Release binaries are always compiled, never taken from a compiler cache.
unset RUSTC_WRAPPER
# cargo-auditable records the dependency tree in the binary, so a scanner reads exactly what was
# compiled in (https://github.com/rust-secure-code/cargo-auditable).
cargo auditable build --release --locked --target "$target" \
  --target-dir "$target_dir" --manifest-path "$root/Cargo.toml" -p zeatend

file="$work/zeatend"
install -m 0755 "$target_dir/$target/release/zeatend" "$file"
# Static: no program interpreter, no shared libraries.
elf="$(readelf --program-headers --dynamic "$file")"
if [[ "$elf" == *INTERP* || "$elf" == *'(NEEDED)'* ]]; then
  echo "zeatend is not a static executable" >&2
  exit 1
fi
[[ "$("$file" --version)" == *"${version#v}"* ]] ||
  { echo "zeatend --version does not report ${version#v}" >&2; exit 1; }
install -m 0644 "$root/LICENSE" "$work/LICENSE"
# Reproducible: sorted entries, the commit's time, no owner names, no gzip timestamp.
tar --format=ustar --sort=name --mtime="@$SOURCE_DATE_EPOCH" \
  --owner=0 --group=0 --numeric-owner -C "$work" -cf - LICENSE zeatend |
  gzip -n -9 >"$output/zeatend-$version-linux-$arch.tar.gz"
ls -l "$output"
