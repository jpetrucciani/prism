# prism implementation status

This file tracks current implementation evidence against the v1 spec. It is intentionally conservative: passing tests prove the listed behavior exists, but unchecked items remain work until covered by direct evidence.

## Current evidence

Local validation last run in this workspace:

```sh
cargo fmt --check
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test --all-features
cargo test --no-default-features
RUSTFLAGS="${RUSTFLAGS:-} -A linker-messages" cargo zigbuild --release --locked --all-features --target x86_64-unknown-linux-musl
python3 scripts/perf_matrix.py --build --iterations 5 --warmups 1 --output target/perf-matrix.md
```

All commands passed. The static build scopes `-A linker-messages` to `cargo zigbuild` because the current zig/lld path emits `ignoring deprecated linker optimization setting '1'` from outside prism's code.

## Covered by executable tests

- Core record framing with newline and NUL separators.
- Positional transform input, stdin transform input, and output rendering with `--json` / `--no-newline`.
- File output creation, append mode, missing-parent errors, and `--mkdir`.
- `dt` epoch formatting, month clamping, DST spring gap, DST fall fold, and invalid timezone errors.
- `dt` timezone default precedence through `PRISM_TZ`, `TZ`, explicit `--tz`, and `--utc`.
- Seeded random output stability, UUID shape, UUID7 helper shape, ULID helper shape, and raw byte output length.
- Exact seeded fixture vectors for `rand --hex`, `--uuid`, `--uuid7`, and `--ulid`; these run in the CI OS matrix.
- `seq`, `repeat`, `pad`, `case`, `slug`, `trim`, `squeeze`, `wrap`, `indent`, `dedent`, `replace`, `field`, `slice`, and `lines` representative behavior.
- Strict field errors, 1-based guidance for field `0`, grapheme slicing, byte slicing with UTF-8 boundary checks, display-width padding, numeric sort, deterministic shuffle.
- `enc` whole-input base64 round trips, per-record decode with `--keep-going`, documented codec smoke coverage, shell encode-only errors, standard base64 `--no-pad` behavior, key codec modifiers, a deterministic binary round-trip corpus, and a deterministic fuzz-style binary corpus.
- `hash` known-answer vectors for every documented algorithm, HMAC-SHA256 vector, raw digest length, short output, and per-line hashing.
- `tpl` variable precedence, required variable errors, dotenv loading, recursive expansion, cycle detection, `--no-gen`, nested helper placeholders, `@now:+offset:%format`, and helper smoke coverage.
- `quote` shell and SQL behavior plus JSON equivalence coverage through codec tests.
- `do` chaining, generator-first seeded stages, native record flow, and alias execution.
- Alias add/show/run/bare dispatch and config default seed/count/no-newline/JSON behavior.
- Conventional `--version` output and extended `prism version` metadata, including target, RNG contract, and wordlist version.
- README/CLI docs stable examples have smoke coverage, `docs/examples.md` is extracted and executed, docs mention the full v1 command surface, and the documentation testing contract points at those executable examples.
- A local performance matrix script covers representative cases across every subtool and writes Markdown timing tables.

## Completion evidence notes

- Seeded-output parity is proven locally on `x86_64-unknown-linux-gnu` and `x86_64-unknown-linux-musl`; CI also runs the same exact-vector tests across Ubuntu and macOS.
