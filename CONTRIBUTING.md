# Contributing

Zeatend is a Kubernetes cluster manager for K3s, RKE2 and KubeSolo over SSH, with a
terminal UI.

## Developer Certificate of Origin

This project uses the [Developer Certificate of Origin 1.1](DCO). Sign off every commit to
certify that you wrote it or otherwise have the right to submit it under the
[Apache License 2.0](LICENSE):

```sh
git commit -s
```

CI rejects pull requests whose commits lack a `Signed-off-by` line matching the author. Commit
messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`,
`docs:`, `ci:`, `chore:`). Pull requests are squash-merged, so the pull request title is the commit
message on `main` and follows the same form. Do not add an AI `Co-Authored-By` trailer.

## Before you open a pull request

Each job in `.github/workflows/ci.yml` runs `nix develop -c scripts/ci/<job>.sh`, so
`nix develop -c scripts/ci/lint.sh` runs a job exactly as CI does. The jobs are `lint`, `test` and
`cluster`. `cluster` installs K3s, RKE2 or KubeSolo on the host, so it runs on a fresh runner in the
merge queue; run it yourself only on a disposable VM.
Underneath:

```sh
cargo fmt --all --check && nixfmt --check flake.nix && shellcheck scripts/*.sh scripts/ci/*.sh && jactionlint
cargo lint                                   # clippy, all targets and features, -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --locked
cargo nextest run --workspace --all-features --locked --no-tests=warn
cargo deny --locked check
```

The pictures of the screens in `docs/screens` are drawn again before a release, not in the pull
request that changes a screen, so two such pull requests do not conflict over them. When
zeabur-oss-bot pushes `release/next`, the release pull request's branch, `.github/workflows/shots.yml`
runs `cargo nextest run -p zeatend --run-ignored only draw_the_pictures_in_docs` and commits the
pictures that changed onto it. Look at them in the release pull request's diff.

A pull request titled `feat`, `fix` or `perf`, or marked breaking with `!`, adds a change file in
`.changeset/` (`knope document-change`) or carries the label `changelog: none`.

## Typed JSON

A known shape, such as a Kubernetes object, the installer's configuration or a journal entry, is a typed
struct (`k8s_openapi` or `#[derive(Deserialize)]`), not a `serde_json::Value`. `clippy.toml` refuses
the `Value` accessors `get`, `get_mut`, `pointer`, `pointer_mut`, `as_object_mut` and
`as_array_mut`. Code that must hold data of any shape puts
`#[expect(clippy::disallowed_methods, reason = "…")]` on the smallest item.

## JSON in tests

- **`json!`, not raw strings.** Tests build JSON with `serde_json::json!`. They compare whole
  objects (`assert_eq!(serde_json::to_value(&typed)?, json!({ … }))`) or deserialize one
  (`serde_json::from_value::<Typed>(json!({ … }))`), and never index into a `Value`.
- **A raw string only where the exact bytes are the subject**: malformed or byte-exact parser
  input, a signature over a body, a golden file, NDJSON. A one-line comment says why.
- **Production code** reads and writes a known shape through a typed struct, not `json!`.

## Releases

A maintainer runs the Release pull request workflow (`.github/workflows/release-pr.yml`). knope
turns the change files in `.changeset/` into the next `CHANGELOG.md` section and version, and
zeabur-oss-bot opens `chore(release): vX.Y.Z` from `release/next`. Once it is merged, the bot tags
it, and `.github/workflows/release.yml` builds static `zeatend` binaries for x86_64 and aarch64
Linux and leaves a draft release with them and their signed checksums. A maintainer publishes the
draft. Running `release.yml` by hand with a version builds the binaries and publishes nothing.

## Toolchain, lints and dependencies

Shared with [Zeabur Builder](https://github.com/zeabur-oss/builder) and
[Zeabur Identity](https://github.com/zeabur-oss/identity):

- `rust-toolchain.toml` sets the compiler: the nightly (by date) and components. `nix develop` builds
  exactly that toolchain, and rustup, which reads the same file, installs it outside Nix. Renovate proposes the new nightly date in `rust-toolchain.toml` (its `rust-toolchain` manager, after the preset's seven-day wait). On that pull request, check that `nix flake update rust-overlay` resolves the date and fix any new clippy lints.
  The date is the same in every zeabur-oss Rust repository, so the four bump pull requests merge together.
- Lints are set once in `Cargo.toml`. All are `warn`, and `cargo lint` turns them into errors.
  Suppress with `#[expect(lint, reason = "…")]`. Only the binary's `output` module writes to
  stdout or stderr.
- `cargo deny check` enforces `deny.toml`: OSI-approved licences, crates.io only, aws-lc-rs as
  the only cryptographic backend, no advisories.
- Cargo does not pick a release younger than seven days (`.cargo/config.toml`).

Documentation is written in English. `README.md` also has `README.zh-TW.md` and
`README.zh-CN.md`: change all three in the same pull request.
