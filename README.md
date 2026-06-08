# prism

[![uses nix](https://img.shields.io/badge/uses-nix-%237EBAE4)](https://nixos.org/)
![rust](https://img.shields.io/badge/Rust-1.98%2B-orange.svg)

`prism` is a static Rust CLI for small text jobs: dates, random strings, case conversion, slugs, fields, slicing, wrapping, encoding, hashing, templates, and linear record pipelines.

It covers the jobs that usually become `printf`, `date`, `sed`, `awk '{print $N}'`, `envsubst`, or a throwaway script. It deliberately does not parse structured data. For JSON, YAML, TOML, CSV records, or schema-aware selection, use [`aq`](https://github.com/jpetrucciani/aq) and pipe the result into or out of `prism`.

## Quickstart

```sh
# Current UTC date in a fixed format
prism dt --utc --fmt '%Y-%m-%dT%H:%M:%SZ'

# Date arithmetic with an explicit timezone
prism dt --tz America/New_York --at 2026-01-31 +1mo

# Deterministic random token
prism --seed release-demo rand --alnum 24

# Generate a padded sequence
prism seq 1..5 --fmt 'item-%03d'

# Normalize a feature branch name
echo '  My Feature: Fix Auth!  ' | prism do 'trim | case snake | slug'

# Select fields with inclusive dot-dot ranges
echo 'alpha beta gamma delta' | prism field 2..3 --osep ,

# Encode each record independently
printf 'one\ntwo\n' | prism enc hex --per-line

# Hash the whole input by default
printf 'hello' | prism hash sha256

# JSON-render output records
printf 'a b\nc d\n' | prism --json field 2

# NUL-framed records for xargs/find-style pipelines
printf 'a\0b\0' | prism -0 case upper

# Render a small template
prism tpl --set SERVICE=api --set PORT=8080 'service=${SERVICE} port=${PORT}'

# Create and run an alias
prism alias add branchname 'trim | case snake | slug'
echo 'My Feature' | prism run branchname

# Extended version metadata
prism version
```

## Install

### Nix development shell

```sh
nix-shell
quality
```

The repo is Nix-first. `default.nix` provides Rust, `cargo-zigbuild`, helper scripts, and the local quality gate.

### Static Linux binary

Inside the Nix shell:

```sh
build_static
```

Outside the Nix shell, with `cargo-zigbuild` available:

```sh
rustup target add x86_64-unknown-linux-musl
RUSTFLAGS="${RUSTFLAGS:-} -A linker-messages" \
  cargo zigbuild --release --locked --all-features --target x86_64-unknown-linux-musl
```

### GitHub release artifacts

GitHub releases attach compressed binaries for Linux, macOS, and Windows plus `SHA256SUMS`. Linux release binaries are built with musl for static linking.

## Core model

Most commands follow the same pipeline:

```text
acquire input -> split into records -> apply verb -> render records -> join records
```

Newline is the default record separator. `-0` switches both input splitting and output joining to NUL. Text verbs require valid UTF-8. Binary modes such as `rand --bytes` and `hash --raw` bypass record framing and do not append a terminator.

Verb classes matter:

| Class                   | Commands                                                                           | Input behavior                                                             |
| ----------------------- | ---------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| Generators              | `dt`, `rand`, `seq`, `repeat`, `pad VALUE`                                         | Do not read stdin.                                                         |
| Per-record transforms   | `case`, `slug`, `slice`, `trim`, `squeeze`, `replace`, `field`, stdin-mode `pad`   | Map over records independently.                                            |
| Whole-stream transforms | `wrap`, `indent`, `dedent`, `lines`, default `enc`, default `hash`, `tpl`, `quote` | Consume the full input as one unit unless the verb documents `--per-line`. |

Rendering flags apply after the verb:

| Flag           | Effect                                                                            |
| -------------- | --------------------------------------------------------------------------------- |
| `--json`       | Render each output record as a JSON string literal.                               |
| `--raw`        | Suppress verb-specific quoting where supported. Mutually exclusive with `--json`. |
| `--no-newline` | Drop only the final record terminator.                                            |

## Environment and config

`prism` intentionally avoids broad ambient `PRISM_*` option overrides. The supported environment inputs are explicit:

| Variable            | Effect                                                                                     |
| ------------------- | ------------------------------------------------------------------------------------------ |
| `PRISM_TZ`          | Default timezone for `prism dt` when `--tz` and `--utc` are not set.                       |
| `TZ`                | Fallback default timezone for `prism dt` when `PRISM_TZ`, `--tz`, and `--utc` are not set. |
| `XDG_CONFIG_HOME`   | Moves the config path used for aliases and `[defaults]`.                                   |
| Process environment | Exposed to `prism tpl` as template variables.                                              |

Config `[defaults]` supports `seed`, `count`, `null`, `no_newline`, `raw`, `json`, `keep_going`, and `quiet`. Explicit CLI flags win over config defaults.

Template variable precedence is `--set KEY=VALUE`, process environment, `--env-file`, then `${VAR:-default}` in the template. Use `--env-file-override` to let dotenv values override process environment values.

## Shell completions

```sh
prism completions bash > ~/.local/share/bash-completion/completions/prism
prism completions zsh > ~/.zfunc/_prism
prism completions fish > ~/.config/fish/completions/prism.fish
```

Supported shells are the shells exposed by clap completions, including `bash`, `zsh`, `fish`, `powershell`, and `elvish`.

## Version metadata

`prism --version` prints the conventional one-line Cargo package version.

`prism version` prints extended release metadata, including package version, target, build profile, build commit when available, deterministic RNG contract, and builtin wordlist version.

## Release gates

Before release, run:

```sh
cargo fmt --check
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test --all-features
cargo test --no-default-features
cargo build --release --locked --all-features
python3 scripts/release_smoke.py --bin target/release/prism
python3 scripts/perf_matrix.py --build --iterations 5 --warmups 1 --output target/perf-matrix.md
```

The GitHub release workflow repeats the release smoke and performance smoke gates, builds Linux/macOS/Windows artifacts, writes checksums, and publishes the release.

## Known limits

- `prism` is text-first. Text verbs reject invalid UTF-8. Binary modes are byte-oriented.
- Templates do not shell out and cannot read files from template syntax, but untrusted templates are not a full sandbox. For untrusted templates, use `--no-gen`, keep recursion off, and run with a clean environment.
- `hash` and `enc` operate on the whole input by default. Use `--per-line` to operate per record.
- `-0` changes record framing only. It does not make text transforms accept arbitrary binary data.
- No locale-aware collation or Turkish-i style special casing. Unicode behavior is deterministic and locale-independent.

## Docs

- [docs/cli.md](docs/cli.md) - command reference.
- [docs/examples.md](docs/examples.md) - stable executable examples.
- [docs/performance.md](docs/performance.md) - repeatable local performance matrix.
- [docs/release.md](docs/release.md) - GitHub release workflow, artifacts, checksums, and local smoke tests.
- [docs/testing.md](docs/testing.md) - test and release acceptance plan.
- [docs/status.md](docs/status.md) - current implementation evidence.

## Non-goals

`prism` does not parse structured data, implement a scripting language, shell out from templates, or provide a full sandbox for untrusted templates.
