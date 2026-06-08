# prism performance matrix

This page defines the local performance smoke matrix for `prism`. It is not a substitute for profiler work, but it gives a repeatable first pass across every subtool and the main I/O modes.

## Running

Inside the Nix shell:

```sh
perf_matrix --iterations 5 --warmups 1 --output target/perf-matrix.md
```

Outside the Nix shell:

```sh
python3 scripts/perf_matrix.py --build --iterations 5 --warmups 1 --output target/perf-matrix.md
```

The script builds or uses `target/release/prism`, writes command output to `/dev/null`, and reports wall-clock timings as a Markdown table. It includes both fixed-overhead cases and throughput cases so startup cost is visible instead of hidden inside large inputs. Each report also includes run metadata: binary path, binary size, build profile, selected groups, and a startup summary when startup cases are included.

For startup overhead specifically, run a larger sample against only the startup group:

```sh
python3 scripts/perf_matrix.py --build --startup-only --iterations 100 --warmups 10 --output target/perf-startup.md
```

You can also filter any group with `--group`, for example `--group records-20k` or repeated `--group codecs-10m --group hashes-10m`.
Use `--group startup-profile` when you want a narrower startup/config decomposition that compares version/help paths, missing config, empty config, defaults config, large config, and alias config.

The default `release` profile is the normal low-overhead build. To compare against the thin-LTO profile, run the same matrix with `--build-profile release-lto`:

```sh
python3 scripts/perf_matrix.py --build --build-profile release-lto --startup-only --iterations 100 --warmups 10 --output target/perf-startup-lto.md
```

## Coverage

The matrix covers:

| Area                    | Cases                                                                                                                         |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| Startup                 | `--version`, `version`, top-level help, subcommand help, no-config, empty-config, defaults-config.                            |
| Startup profiling       | Version/help paths plus one-record transforms under no config, empty config, defaults config, large config, and alias config. |
| Tiny inputs             | One-record transforms, tiny codecs/hashes, one-record chains, one-record aliases.                                             |
| Generators              | `dt`, `rand`, `seq`, `repeat`, UUIDv7, raw byte generation, 1k to 10k generated records.                                      |
| Per-record transforms   | `pad`, `case`, `slug`, `trim`, `squeeze`, `replace`, `field`, `slice`, newline and NUL framing, 100 to 100k records.          |
| Whole-stream transforms | `wrap`, `indent`, `dedent`, `lines` numbering, uniq, global uniq, sort, shuffle.                                              |
| Codecs                  | Base64, base64url, hex, ascii85, JSON, URL component, per-line hex, 1 MiB and 10 MiB inputs.                                  |
| Hashes                  | SHA-256, SHA-512, BLAKE3, xxh3, crc32, per-line SHA-256, 1 MiB and 10 MiB inputs.                                             |
| Rendering and files     | `--json`, `--no-newline`, `quote json`, atomic `--out` writes.                                                                |
| Templates               | `tpl` variable expansion, helpers, recursive expansion.                                                                       |
| Composition             | `do` chains, `run` aliases, alias config, and config-default overhead.                                                        |

## Interpreting results

Use the median column for fixed overhead and rough comparison. Min/max columns are included to catch scheduler noise or outliers. `MiB/s` and `Records/s` are derived from median runtime and are more useful for larger throughput cases. Startup and tiny-input cases usually show process-launch, config, and Clap overhead more than transformation cost.

When changing implementation internals, keep the old and new result tables in the PR description rather than committing machine-specific timing changes.
