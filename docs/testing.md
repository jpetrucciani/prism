# prism testing and acceptance plan

This file defines the test obligations for the v1 spec. A feature is complete only when the relevant tests exist, run in CI, and document the behavior they protect.

## Test tiers

| Tier                 | Purpose                                                                      | Required for                                                    |
| -------------------- | ---------------------------------------------------------------------------- | --------------------------------------------------------------- |
| Unit tests           | Parser, formatter, field spec, offset arithmetic, escaping, hashing helpers. | Every non-trivial helper.                                       |
| Golden CLI tests     | Command input, flags, stdout, stderr, and exit code.                         | Every verb and global flag interaction.                         |
| Property tests       | Invariants over broad generated input.                                       | Codecs, field ranges, record framing, seeded determinism.       |
| Known-answer vectors | Compatibility with external standards.                                       | Hashes, encodings, UUID/ULID formatting, datetime examples.     |
| Filesystem tests     | Output atomicity and config behavior.                                        | `-o`, aliases, config, dotenv, env files.                       |
| Cross-platform tests | Determinism and static-build confidence.                                     | Seeded random output and release builds.                        |
| Docs examples        | Prevent stale examples.                                                      | README, CLI docs, and spec examples once implementation exists. |
| Performance matrix   | Track broad-path wall-clock behavior during development.                     | Representative cases across every subtool.                      |

## Required CI gates

The CI gate for Rust changes is:

```sh
cargo fmt --check
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test --all-features
cargo test --no-default-features
```

Release-readiness CI also needs:

```sh
RUSTFLAGS="${RUSTFLAGS:-} -A linker-messages" cargo zigbuild --release --locked --all-features --target x86_64-unknown-linux-musl
python3 scripts/release_smoke.py --bin target/release/prism
```

Seeded randomness must run on at least two architectures or OS targets and compare byte-for-byte expected outputs.

Performance smoke coverage is local-only by default:

```sh
perf_matrix --iterations 5 --warmups 1 --output target/perf-matrix.md
```

The matrix and interpretation rules are documented in [docs/performance.md](performance.md).

The release workflow runs the release smoke script, a performance smoke matrix, platform builds for Linux/macOS/Windows, and checksum generation before publishing artifacts.

## Golden CLI test shape

Every CLI golden test records:

- Command argv.
- Stdin bytes.
- Relevant env vars.
- Config fixture path, if any.
- Expected stdout bytes.
- Expected stderr bytes or pattern.
- Expected exit code.
- Expected output file bytes and permissions for `-o` tests.

Golden tests should prefer byte assertions over string assertions when record separators, NUL mode, or binary behavior matters.

## Global flag coverage

| Area           | Required cases                                                                                                                      |
| -------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `-i`           | Reads file input, `-` reads stdin, positional input wins for transforms.                                                            |
| `-o`           | Atomic success, fail-fast cleanup, existing mode preservation, new-file umask behavior where portable, missing parent error.        |
| `--append`     | Appends without replacing existing content.                                                                                         |
| `--mkdir`      | Creates missing parent directories for `-o`.                                                                                        |
| `--mode`       | Applies requested mode on create/replace.                                                                                           |
| `-n`           | Works for generators, errors for non-generator first stages in chains, no-op or usage behavior matches command docs outside chains. |
| `--seed`       | Stable output for every seeded command.                                                                                             |
| `-0`           | Splits and joins on NUL, preserves interior empty records.                                                                          |
| `--no-newline` | Removes only final terminator.                                                                                                      |
| `--raw`        | Suppresses verb-specific quoting where supported.                                                                                   |
| `--json`       | JSON-renders every text record and rejects `--raw`.                                                                                 |
| `--keep-going` | Emits successes, logs failures, exits nonzero when any record fails.                                                                |
| `--quiet`      | Suppresses non-fatal notes, especially binary-mode rendering notes.                                                                 |

## I/O model tests

Required cases:

- Empty input yields zero records for per-record transforms.
- Final trailing separator is not treated as an extra empty record.
- Interior empty records are preserved.
- Newline and NUL framing behave identically except for separator byte.
- Whole-stream commands consume the full byte/text input by default.
- `enc --per-line` and `hash --per-line` operate per record.
- Binary modes bypass framing and never append terminators.
- Invalid UTF-8 fails in text verbs and is accepted in binary modes.

## Command acceptance matrix

### `dt`

Golden tests:

- RFC3339 local default with fixed clock fixture.
- `--utc`, `--tz`, `--fmt`, `--iso`, `--epoch`, `--epoch-ms`, `--rfc2822`.
- `--from` seconds and milliseconds.
- Offset chains such as `+2h30m` and `-1w2d`.
- Month clamp: Jan 31 `+1mo`.
- Leap year clamp: Jan 31 2024 `+1mo` -> Feb 29 2024.
- Spring-forward invalid local time shifts forward by gap width.
- Fall-back ambiguous local time chooses earlier occurrence.
- Invalid timezone returns usage or runtime error as specified by parser layer.

### `rand`

Golden tests:

- Each text mode with fixed seed.
- `--count` emits multiple records.
- `--bytes` emits exact byte count and no terminator.
- `uuid` version and variant bits.
- `uuid7` timestamp bits under `--seed` and `--now`.
- `ulid` length, alphabet, and deterministic timestamp behavior.
- `--wordlist` fixture controls word output.

Property tests:

- Generated characters are always in the selected alphabet.
- Same seed and args produce identical bytes.
- Different args consume RNG in documented order and do not accidentally alias outputs.

### `seq`

Golden tests:

- Numeric ascending and descending ranges.
- Alphabetic ranges.
- Padding.
- `--fmt`.
- `--hex`.
- `--sep` single-record join.
- Invalid ranges produce usage errors.

### `repeat`

Golden tests:

- Plain repetition.
- Repetition with separator.
- Zero repetitions.
- Large count sanity without quadratic behavior.

### `pad`

Golden tests:

- Left, right, and center padding.
- Custom fill.
- Positional generator mode.
- Stdin per-record mode.
- Display width with combining marks and CJK.
- `--width-mode chars|bytes|display`.

### `case`

Golden tests:

- `snake`, `camel`, `pascal`, `kebab`, `scream`, `const`, `title`, `upper`, `lower`, `swap`, `dot`, `path`.
- Camel-case boundaries.
- Punctuation and whitespace boundaries.
- Non-ASCII default case mappings.
- No locale-special behavior.

### `slug`

Golden tests:

- ASCII punctuation collapse.
- NFKD and combining mark removal.
- Separator override.
- Max length.
- `--unicode` preservation.
- Empty result behavior.

### `trim` and `squeeze`

Golden tests:

- Unicode whitespace trimming.
- `--ascii` trimming.
- Left and right modes.
- Explicit trim chars.
- Whitespace squeeze.
- Repeated char squeeze.

### `wrap`, `indent`, and `dedent`

Golden tests:

- Wrap width.
- Hanging indent.
- Existing paragraph boundaries.
- Spaces and tabs indentation.
- Common whitespace dedent.
- Display-width handling.

### `replace`

Golden tests:

- Literal all occurrences.
- Literal first occurrence.
- Regex replacement.
- Capture references.
- Invalid regex usage error.
- Replacement over multiple records.

### `field`

Golden tests:

- Default whitespace delimiter.
- Literal delimiter.
- Regex delimiter.
- Single index.
- Multiple indices.
- Inclusive ranges.
- Open ranges.
- Negative indices.
- Missing fields as empty strings.
- Wholly out-of-range ranges emit nothing.
- `--strict-fields` errors.
- Bare `0` usage error with 1-based guidance.

Property tests:

- `1..-1` reconstructs all fields for delimiter-free field values.
- Negative indices match equivalent positive indices for known field counts.

### `slice`

Golden tests:

- Scalar index and ranges.
- Negative bounds.
- Byte mode.
- Grapheme mode.
- Invalid byte boundaries.
- Empty and out-of-range behavior.

### `lines`

Golden tests:

- Numbering.
- Adjacent uniq.
- Global uniq preserving first-seen order.
- Reverse.
- Seeded shuffle.
- Lexical sort.
- Numeric sort.
- Reverse sort.
- NUL mode.

### `enc`

Golden tests:

- Known examples for every codec.
- Decode known examples for every decode-capable codec.
- `shell` encode-only decode error.
- `--no-pad`, `--upper`, and `--component` modifiers.
- Whole-input default with embedded newlines.
- `--per-line` behavior.
- Decode failure fail-fast.
- Decode failure with `--keep-going`.

Property tests:

- `decode(encode(input)) == input` for every decode-capable binary-safe codec.
- Text-only codecs round-trip over valid text domains.
- URL component encoding does not leave unsafe component bytes unescaped.

### `hash`

Known-answer vectors:

- Empty input for every algorithm.
- `abc` for every algorithm.
- Multi-record input hashed whole by default.
- Per-line mode hashes records independently.
- HMAC vectors from published digest test vectors.
- `--upper`, `--base64`, `--short`, and `--raw` output modes.
- `-i` file bytes are hashed exactly.

### `tpl`

Golden tests:

- Basic `${VAR}`.
- Default `${VAR:-default}`.
- Required `${VAR:?message}` exit code `3`.
- Alternate `${VAR:+alt}`.
- `--strict` missing var.
- `--set` precedence.
- Process env precedence.
- Dotenv precedence.
- `--env-file-override`.
- `@` helpers.
- `--no-gen` rejects or leaves helpers according to implementation docs.
- Recursive expansion to fixpoint.
- Cycle detection exit code `4`.
- Max-depth overflow exit code `4`.
- Error names offending key or placeholder.

Security-oriented tests:

- Templates do not shell out.
- Templates do not read files by placeholder syntax.
- Hardened mode with clean env does not expose unrelated process variables.

### `quote`

Golden tests:

- Shell single-argument quoting with spaces, quotes, and metacharacters.
- JSON literal equivalence with `enc json`.
- C/Rust string escaping.
- Regex metacharacter escaping.
- SQL single-quote escaping.

### `do` chains

Golden tests:

- Per-record pipeline.
- Generator as first stage.
- Generator as later stage usage error.
- Whole-stream stage collapsing records.
- Outer `-i`, `-o`, `-s`, `-0`, and `-n` semantics.
- Native record passing without intermediate serialization bugs.
- Shell-word quoting inside chain expressions.

### Aliases and config

Filesystem tests:

- Missing config is success.
- `alias path` prints resolved path.
- `alias add`, `show`, `list`, and `rm` mutate config correctly.
- `run` executes aliases.
- `x` executes aliases.
- Positional args after alias name pass to first stage.
- Built-ins win bare-name collisions.
- `expand_bare = true` enables bare dispatch.
- Flag precedence follows CLI, alias, config default, built-in default.
- Invalid alias chain fails with usage error.

## Documentation tests

Executable examples live in `docs/examples.md` using fenced `prism-example` blocks. CI extracts those blocks and runs them through the compiled binary.

Reference examples in `README.md`, `SPEC.md`, and `docs/cli.md` are syntax-oriented examples. They are backed by the CLI integration matrix and docs contract tests rather than executed verbatim, because many intentionally demonstrate current-time commands, shell redirection, or user-local files.

Docs tests should run executable examples in a temp directory with isolated config and environment. Examples involving current time should use fixed-clock test support or explicit `--at`/`--from` anchors.

## Release acceptance checklist

A release candidate is not ready until:

- All required CI gates pass.
- Every command in `docs/cli.md` has at least one golden test.
- Every behavior in `SPEC.md` that mentions an error or precedence rule has a direct test.
- The seeded randomness cross-platform fixture passes on two targets.
- The musl static binary build succeeds.
- `--version` prints the prism version, target, RNG contract version, and wordlist version.
- README examples match actual output.
- `TODO.md` has no unchecked task for the target milestone.
