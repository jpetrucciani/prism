#!/usr/bin/env python3
"""Run a broad prism CLI performance matrix and emit Markdown."""

from __future__ import annotations

import argparse
import os
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class PerfCase:
    group: str
    name: str
    args: tuple[str, ...]
    stdin: bytes
    records: int | None = None
    env: dict[str, str] | None = None


@dataclass(frozen=True)
class PerfResult:
    case: PerfCase
    command: str
    input_bytes: int
    median_ms: float
    min_ms: float
    max_ms: float
    runs: int


@dataclass(frozen=True)
class RunMetadata:
    binary: Path
    binary_size_bytes: int
    build_profile: str
    groups: tuple[str, ...]
    warmups: int
    iterations: int


def repeated_lines(line: str, count: int) -> bytes:
    return ((line + "\n") * count).encode()


def nul_records(record: str, count: int) -> bytes:
    return ((record + "\0") * count).encode()


def binary_blob(size: int) -> bytes:
    pattern = bytes(range(256))
    full, rem = divmod(size, len(pattern))
    return pattern * full + pattern[:rem]


def text_blob(size: int) -> bytes:
    pattern = (
        "Prism performance text with spaces, punctuation, cafe, quotes, and slashes. "
    ).encode()
    full, rem = divmod(size, len(pattern))
    return pattern * full + pattern[:rem]


def write_config(path: Path, body: str) -> dict[str, str]:
    config_dir = path / "prism"
    config_dir.mkdir(parents=True, exist_ok=True)
    (config_dir / "config.toml").write_text(body, encoding="utf-8")
    return {"XDG_CONFIG_HOME": str(path)}


def build_cases(temp: Path) -> list[PerfCase]:
    no_config_env = {"XDG_CONFIG_HOME": str(temp / "no-config")}
    empty_config_env = write_config(temp / "empty-config", "")
    alias_env = write_config(
        temp / "alias-config",
        '[alias]\nfastslug = "trim | case snake | slug"\n',
    )
    defaults_env = write_config(
        temp / "defaults-config",
        '[defaults]\nseed = "perf"\nno_newline = true\n',
    )
    large_alias_entries = "\n".join(
        f'alias_{idx} = "trim | case snake | slug"' for idx in range(500)
    )
    large_config_env = write_config(
        temp / "large-config",
        f'[alias]\nfastslug = "trim | case snake | slug"\n{large_alias_entries}\n'
        '[defaults]\nseed = "perf"\nno_newline = true\n',
    )
    out_dir = temp / "out"
    out_dir.mkdir(parents=True, exist_ok=True)

    line_100 = repeated_lines("  HelloWorld value_123 field-two field-three  ", 100)
    line_20k = repeated_lines("  HelloWorld value_123 field-two field-three  ", 20_000)
    line_100k = repeated_lines(
        "  HelloWorld value_123 field-two field-three  ", 100_000
    )
    csv_20k = repeated_lines("alpha,beta,gamma,delta,epsilon", 20_000)
    csv_100k = repeated_lines("alpha,beta,gamma,delta,epsilon", 100_000)
    regex_20k = repeated_lines("user-123 paid 456 credits", 20_000)
    regex_100k = repeated_lines("user-123 paid 456 credits", 100_000)
    numeric_20k = repeated_lines("100\n3\n42\n7\n13", 4_000)
    numeric_100k = repeated_lines("100\n3\n42\n7\n13", 20_000)
    dup_100k = repeated_lines("alpha\nalpha\nbeta\ngamma\ngamma", 20_000)
    prose_512k = (
        "Prism wraps this sentence across a display width while preserving words. "
        * 7_200
    ).encode()
    indented_20k = repeated_lines("        block value", 20_000)
    indented_100k = repeated_lines("        block value", 100_000)
    nul_20k = nul_records("hello world", 20_000)
    template_5k = (
        "service=${SERVICE:-api} port=${PORT:-8080} id=${@rand:hex:8}\n" * 5_000
    ).encode()
    template_recursive_2k = ("${A}\n" * 2_000).encode()
    one_kib = text_blob(1024)
    one_mib = binary_blob(1024 * 1024)
    ten_mib = binary_blob(10 * 1024 * 1024)
    text_one_mib = text_blob(1024 * 1024)
    text_ten_mib = text_blob(10 * 1024 * 1024)

    cases = [
        # Startup and tiny-input fixed-cost cases.
        PerfCase(
            "startup", "version_flag_no_config", ("--version",), b"", env=no_config_env
        ),
        PerfCase(
            "startup",
            "version_metadata_no_config",
            ("version",),
            b"",
            env=no_config_env,
        ),
        PerfCase("startup", "top_help_no_config", ("--help",), b"", env=no_config_env),
        PerfCase(
            "startup", "sub_help_no_config", ("rand", "--help"), b"", env=no_config_env
        ),
        PerfCase(
            "startup",
            "version_metadata_empty_config",
            ("version",),
            b"",
            env=empty_config_env,
        ),
        PerfCase(
            "startup",
            "version_metadata_defaults_config",
            ("version",),
            b"",
            env=defaults_env,
        ),
        PerfCase(
            "startup-profile",
            "version_flag_no_config",
            ("--version",),
            b"",
            env=no_config_env,
        ),
        PerfCase(
            "startup-profile",
            "version_subcommand_no_config",
            ("version",),
            b"",
            env=no_config_env,
        ),
        PerfCase(
            "startup-profile", "top_help_no_config", ("--help",), b"", env=no_config_env
        ),
        PerfCase(
            "startup-profile",
            "sub_help_no_config",
            ("rand", "--help"),
            b"",
            env=no_config_env,
        ),
        PerfCase(
            "startup-profile",
            "trim_no_config",
            ("trim",),
            b"  hello  \n",
            records=1,
            env=no_config_env,
        ),
        PerfCase(
            "startup-profile",
            "trim_empty_config",
            ("trim",),
            b"  hello  \n",
            records=1,
            env=empty_config_env,
        ),
        PerfCase(
            "startup-profile",
            "trim_defaults_config",
            ("trim",),
            b"  hello  \n",
            records=1,
            env=defaults_env,
        ),
        PerfCase(
            "startup-profile",
            "trim_large_config",
            ("trim",),
            b"  hello  \n",
            records=1,
            env=large_config_env,
        ),
        PerfCase(
            "startup-profile",
            "run_alias_small_config",
            ("run", "fastslug"),
            b"  hello  \n",
            records=1,
            env=alias_env,
        ),
        PerfCase(
            "startup-profile",
            "run_alias_large_config",
            ("run", "fastslug"),
            b"  hello  \n",
            records=1,
            env=large_config_env,
        ),
        PerfCase(
            "tiny",
            "dt_one_fixed",
            ("dt", "--utc", "--from", "0", "--fmt", "%FT%TZ"),
            b"",
            records=1,
            env=no_config_env,
        ),
        PerfCase(
            "tiny",
            "rand_uuid_seeded",
            ("--seed", "perf", "rand", "--uuid"),
            b"",
            records=1,
            env=no_config_env,
        ),
        PerfCase(
            "tiny", "seq_ten", ("seq", "1..10"), b"", records=10, env=no_config_env
        ),
        PerfCase(
            "tiny",
            "repeat_ten",
            ("repeat", "abc", "10", "--sep", ","),
            b"",
            records=10,
            env=no_config_env,
        ),
        PerfCase(
            "tiny", "trim_one", ("trim",), b"  hello  \n", records=1, env=no_config_env
        ),
        PerfCase(
            "tiny",
            "case_one",
            ("case", "snake"),
            b"HelloWorld\n",
            records=1,
            env=no_config_env,
        ),
        PerfCase(
            "tiny",
            "slug_one",
            ("slug",),
            b"Hello, World!\n",
            records=1,
            env=no_config_env,
        ),
        PerfCase(
            "tiny",
            "field_one",
            ("field", "2..3"),
            b"alpha beta gamma\n",
            records=1,
            env=no_config_env,
        ),
        PerfCase(
            "tiny",
            "replace_regex_one",
            ("replace", "--regex", "\\d+", "N"),
            b"user-123\n",
            records=1,
            env=no_config_env,
        ),
        PerfCase(
            "tiny", "enc_base64_1k", ("enc", "base64"), one_kib, env=no_config_env
        ),
        PerfCase(
            "tiny", "hash_sha256_1k", ("hash", "sha256"), one_kib, env=no_config_env
        ),
        PerfCase(
            "tiny",
            "quote_shell_one",
            ("quote", "shell"),
            b"hello world",
            env=no_config_env,
        ),
        PerfCase(
            "tiny",
            "tpl_one_set",
            ("tpl", "--set", "PORT=8080"),
            b"port=${PORT}\n",
            env=no_config_env,
        ),
        PerfCase(
            "tiny",
            "do_one_chain",
            ("do", "trim | case snake | slug"),
            b"  Hello World  \n",
            records=1,
            env=no_config_env,
        ),
        PerfCase(
            "tiny",
            "alias_one_chain",
            ("run", "fastslug"),
            b"  Hello World  \n",
            records=1,
            env=alias_env,
        ),
        # Generator cases.
        PerfCase(
            "generators",
            "dt_1k_fmt_utc",
            ("-n", "1000", "dt", "--utc", "--from", "0", "--fmt", "%FT%TZ"),
            b"",
            records=1_000,
            env=no_config_env,
        ),
        PerfCase(
            "generators",
            "dt_10k_epoch",
            ("-n", "10000", "dt", "--utc", "--from", "0", "--epoch"),
            b"",
            records=10_000,
            env=no_config_env,
        ),
        PerfCase(
            "generators",
            "rand_1k_alnum32",
            ("--seed", "perf", "-n", "1000", "rand", "--alnum", "32"),
            b"",
            records=1_000,
            env=no_config_env,
        ),
        PerfCase(
            "generators",
            "rand_10k_digits8",
            ("--seed", "perf", "-n", "10000", "rand", "--digits", "8"),
            b"",
            records=10_000,
            env=no_config_env,
        ),
        PerfCase(
            "generators",
            "rand_1k_uuid7",
            ("--seed", "perf", "-n", "1000", "rand", "--uuid7"),
            b"",
            records=1_000,
            env=no_config_env,
        ),
        PerfCase(
            "generators",
            "rand_bytes_1m",
            ("--seed", "perf", "rand", "--bytes", str(1024 * 1024)),
            b"",
            env=no_config_env,
        ),
        PerfCase(
            "generators",
            "rand_bytes_10m",
            ("--seed", "perf", "rand", "--bytes", str(10 * 1024 * 1024)),
            b"",
            env=no_config_env,
        ),
        PerfCase(
            "generators",
            "seq_10k_numeric",
            ("seq", "1..10000"),
            b"",
            records=10_000,
            env=no_config_env,
        ),
        PerfCase(
            "generators",
            "seq_10k_padded",
            ("seq", "1..10000", "--pad", "8"),
            b"",
            records=10_000,
            env=no_config_env,
        ),
        PerfCase(
            "generators",
            "repeat_10k_sep",
            ("repeat", "abc", "10000", "--sep", ","),
            b"",
            records=10_000,
            env=no_config_env,
        ),
        # Per-record text transforms.
        PerfCase(
            "records-100",
            "trim_100",
            ("trim",),
            line_100,
            records=100,
            env=no_config_env,
        ),
        PerfCase(
            "records-100",
            "case_100_snake",
            ("case", "snake"),
            line_100,
            records=100,
            env=no_config_env,
        ),
        PerfCase(
            "records-100",
            "replace_100_regex",
            ("replace", "--regex", "\\d+", "N"),
            regex_20k[:2_600],
            records=100,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "pad_20k_display",
            ("pad", "--right", "80"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "case_20k_snake",
            ("case", "snake"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "case_20k_upper",
            ("case", "upper"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "slug_20k_ascii",
            ("slug",),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "trim_20k_unicode",
            ("trim",),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "squeeze_20k_space",
            ("squeeze",),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "squeeze_20k_char",
            ("squeeze", "--char", "l"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "replace_20k_literal",
            ("replace", "value", "item"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "replace_20k_regex",
            ("replace", "--regex", "\\d+", "N"),
            regex_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "field_20k_range",
            ("field", "2..4", "-d", ",", "--osep", "|"),
            csv_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "field_20k_regex_delim",
            ("field", "2..4", "-d", "[,:]", "--regex", "--osep", "|"),
            csv_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "slice_20k_chars",
            ("slice", "2..12"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "slice_20k_bytes",
            ("slice", "--bytes", "2..12"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-20k",
            "null_case_20k_upper",
            ("-0", "case", "upper"),
            nul_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-100k",
            "trim_100k_unicode",
            ("trim",),
            line_100k,
            records=100_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-100k",
            "case_100k_snake",
            ("case", "snake"),
            line_100k,
            records=100_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-100k",
            "replace_100k_regex",
            ("replace", "--regex", "\\d+", "N"),
            regex_100k,
            records=100_000,
            env=no_config_env,
        ),
        PerfCase(
            "records-100k",
            "field_100k_range",
            ("field", "2..4", "-d", ",", "--osep", "|"),
            csv_100k,
            records=100_000,
            env=no_config_env,
        ),
        # Whole-stream transforms.
        PerfCase(
            "whole-stream",
            "wrap_prose_width72",
            ("wrap", "--width", "72"),
            prose_512k,
            env=no_config_env,
        ),
        PerfCase(
            "whole-stream",
            "indent_20k_spaces",
            ("indent", "--spaces", "4"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "whole-stream",
            "dedent_20k_common",
            ("dedent",),
            indented_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "whole-stream",
            "dedent_100k_common",
            ("dedent",),
            indented_100k,
            records=100_000,
            env=no_config_env,
        ),
        PerfCase(
            "lines",
            "lines_number_20k",
            ("lines", "--number"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "lines",
            "lines_uniq_100k",
            ("lines", "--uniq"),
            dup_100k,
            records=100_000,
            env=no_config_env,
        ),
        PerfCase(
            "lines",
            "lines_uniq_global_100k",
            ("lines", "--uniq-global"),
            dup_100k,
            records=100_000,
            env=no_config_env,
        ),
        PerfCase(
            "lines",
            "lines_sort_numeric_20k",
            ("lines", "--sort", "--numeric"),
            numeric_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "lines",
            "lines_sort_numeric_100k",
            ("lines", "--sort", "--numeric"),
            numeric_100k,
            records=100_000,
            env=no_config_env,
        ),
        PerfCase(
            "lines",
            "lines_shuffle_20k",
            ("--seed", "perf", "lines", "--shuffle"),
            numeric_20k,
            records=20_000,
            env=no_config_env,
        ),
        # Codecs, hashes, and binary/text bulk operations.
        PerfCase(
            "codecs-1m", "enc_base64_1m", ("enc", "base64"), one_mib, env=no_config_env
        ),
        PerfCase(
            "codecs-1m",
            "enc_base64url_1m",
            ("enc", "base64url"),
            one_mib,
            env=no_config_env,
        ),
        PerfCase("codecs-1m", "enc_hex_1m", ("enc", "hex"), one_mib, env=no_config_env),
        PerfCase(
            "codecs-1m",
            "enc_ascii85_1m",
            ("enc", "ascii85"),
            one_mib,
            env=no_config_env,
        ),
        PerfCase(
            "codecs-1m", "enc_json_1m", ("enc", "json"), text_one_mib, env=no_config_env
        ),
        PerfCase(
            "codecs-1m",
            "enc_url_component_1m",
            ("enc", "url", "--component"),
            text_one_mib,
            env=no_config_env,
        ),
        PerfCase(
            "codecs-10m",
            "enc_base64_10m",
            ("enc", "base64"),
            ten_mib,
            env=no_config_env,
        ),
        PerfCase(
            "codecs-10m", "enc_hex_10m", ("enc", "hex"), ten_mib, env=no_config_env
        ),
        PerfCase(
            "codecs-lines",
            "enc_hex_per_line_20k",
            ("enc", "hex", "--per-line"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "hashes-1m",
            "hash_sha256_1m",
            ("hash", "sha256"),
            one_mib,
            env=no_config_env,
        ),
        PerfCase(
            "hashes-1m",
            "hash_sha512_1m",
            ("hash", "sha512"),
            one_mib,
            env=no_config_env,
        ),
        PerfCase(
            "hashes-1m",
            "hash_blake3_1m",
            ("hash", "blake3"),
            one_mib,
            env=no_config_env,
        ),
        PerfCase(
            "hashes-1m", "hash_xxh3_1m", ("hash", "xxh3"), one_mib, env=no_config_env
        ),
        PerfCase(
            "hashes-1m", "hash_crc32_1m", ("hash", "crc32"), one_mib, env=no_config_env
        ),
        PerfCase(
            "hashes-10m",
            "hash_sha256_10m",
            ("hash", "sha256"),
            ten_mib,
            env=no_config_env,
        ),
        PerfCase(
            "hashes-10m",
            "hash_blake3_10m",
            ("hash", "blake3"),
            ten_mib,
            env=no_config_env,
        ),
        PerfCase(
            "hashes-lines",
            "hash_sha256_per_line_20k",
            ("hash", "sha256", "--per-line"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        # Rendering, files, templates, and composition.
        PerfCase(
            "rendering",
            "json_render_trim_20k",
            ("--json", "trim"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "rendering",
            "no_newline_hash_1m",
            ("--no-newline", "hash", "sha256"),
            one_mib,
            env=no_config_env,
        ),
        PerfCase(
            "rendering",
            "quote_json_1m",
            ("quote", "json"),
            text_one_mib,
            env=no_config_env,
        ),
        PerfCase(
            "files",
            "out_hash_1m_atomic",
            ("--out", str(out_dir / "hash.txt"), "hash", "sha256"),
            one_mib,
            env=no_config_env,
        ),
        PerfCase(
            "files",
            "out_case_20k_atomic",
            ("--out", str(out_dir / "case.txt"), "case", "upper"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "templates",
            "tpl_5k_vars_helpers",
            ("--seed", "perf", "tpl", "--set", "SERVICE=api"),
            template_5k,
            records=5_000,
            env=no_config_env,
        ),
        PerfCase(
            "templates",
            "tpl_2k_recursive",
            ("tpl", "--recursive", "--set", "A=${B}", "--set", "B=ok"),
            template_recursive_2k,
            records=2_000,
            env=no_config_env,
        ),
        PerfCase(
            "composition",
            "do_20k_chain",
            ("do", "trim | case snake | slug"),
            line_20k,
            records=20_000,
            env=no_config_env,
        ),
        PerfCase(
            "composition",
            "do_100k_light_chain",
            ("do", "trim | case upper"),
            line_100k,
            records=100_000,
            env=no_config_env,
        ),
        PerfCase(
            "composition",
            "alias_20k_run",
            ("run", "fastslug"),
            line_20k,
            records=20_000,
            env=alias_env,
        ),
        PerfCase(
            "composition",
            "defaults_config_rand",
            ("rand", "--hex", "8"),
            b"",
            records=1,
            env=defaults_env,
        ),
    ]
    return cases


def build_binary(profile: str) -> None:
    subprocess.run(
        ["cargo", "build", "--locked", "--all-features", "--profile", profile],
        check=True,
    )


def run_case(binary: Path, case: PerfCase, warmups: int, iterations: int) -> PerfResult:
    command = [str(binary), *case.args]
    env = os.environ.copy()
    if case.env:
        env.update(case.env)
    for _ in range(warmups):
        subprocess.run(
            command,
            input=case.stdin,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            env=env,
            check=True,
        )

    timings_ms: list[float] = []
    for _ in range(iterations):
        start = time.perf_counter_ns()
        subprocess.run(
            command,
            input=case.stdin,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            env=env,
            check=True,
        )
        elapsed = time.perf_counter_ns() - start
        timings_ms.append(elapsed / 1_000_000)

    return PerfResult(
        case=case,
        command="prism " + " ".join(case.args),
        input_bytes=len(case.stdin),
        median_ms=statistics.median(timings_ms),
        min_ms=min(timings_ms),
        max_ms=max(timings_ms),
        runs=iterations,
    )


def mib_per_second(input_bytes: int, median_ms: float) -> str:
    if input_bytes == 0 or median_ms == 0:
        return "-"
    return f"{(input_bytes / 1_048_576) / (median_ms / 1000):.1f}"


def records_per_second(records: int | None, median_ms: float) -> str:
    if records is None or median_ms == 0:
        return "-"
    return f"{records / (median_ms / 1000):.0f}"


def format_size(size_bytes: int) -> str:
    size = float(size_bytes)
    for unit in ("B", "KiB", "MiB", "GiB"):
        if size < 1024 or unit == "GiB":
            return f"{size:.1f} {unit}" if unit != "B" else f"{size_bytes} B"
        size /= 1024
    return f"{size_bytes} B"


def render_metadata(metadata: RunMetadata, results: list[PerfResult]) -> list[str]:
    groups = ", ".join(metadata.groups)
    lines = [
        "# prism performance matrix",
        "",
        "## Run metadata",
        "",
        f"- Binary: `{metadata.binary}`",
        f"- Binary size: {format_size(metadata.binary_size_bytes)} ({metadata.binary_size_bytes} bytes)",
        f"- Cargo build profile: `{metadata.build_profile}`",
        f"- Groups: `{groups}`",
        f"- Warmups per case: {metadata.warmups}",
        f"- Measured runs per case: {metadata.iterations}",
    ]
    startup = [result for result in results if result.case.group == "startup"]
    if startup:
        startup_medians = [result.median_ms for result in startup]
        fastest = min(startup, key=lambda result: result.median_ms)
        slowest = max(startup, key=lambda result: result.median_ms)
        lines.extend(
            [
                f"- Startup case median: {statistics.median(startup_medians):.3f} ms across {len(startup)} cases",
                f"- Fastest startup case: `{fastest.case.name}` at {fastest.median_ms:.3f} ms",
                f"- Slowest startup case: `{slowest.case.name}` at {slowest.median_ms:.3f} ms",
            ]
        )
    else:
        lines.append("- Startup cases: not included")
    lines.extend(["", "## Results", ""])
    return lines


def render_markdown(results: list[PerfResult], metadata: RunMetadata) -> str:
    lines = render_metadata(metadata, results)
    lines.extend(
        [
            "| Group | Case | Command | Input bytes | Records | Median ms | Min ms | Max ms | MiB/s | Records/s | Runs |",
            "| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    for result in results:
        records = "-" if result.case.records is None else str(result.case.records)
        lines.append(
            "| {group} | {name} | `{command}` | {input_bytes} | {records} | {median:.3f} | {minimum:.3f} | {maximum:.3f} | {mib_s} | {records_s} | {runs} |".format(
                group=result.case.group,
                name=result.case.name,
                command=result.command.replace("|", "\\|"),
                input_bytes=result.input_bytes,
                records=records,
                median=result.median_ms,
                minimum=result.min_ms,
                maximum=result.max_ms,
                mib_s=mib_per_second(result.input_bytes, result.median_ms),
                records_s=records_per_second(result.case.records, result.median_ms),
                runs=result.runs,
            )
        )
    return "\n".join(lines) + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run prism performance matrix")
    parser.add_argument("--bin", type=Path, help="Path to prism binary")
    parser.add_argument(
        "--build", action="store_true", help="Build binary before timing"
    )
    parser.add_argument(
        "--build-profile",
        default="release",
        help="Cargo profile used by --build and as the default target path",
    )
    parser.add_argument(
        "--group", action="append", help="Run only this group. May be repeated"
    )
    parser.add_argument(
        "--startup-only", action="store_true", help="Shortcut for --group startup"
    )
    parser.add_argument(
        "--iterations", type=int, default=5, help="Measured runs per case"
    )
    parser.add_argument("--warmups", type=int, default=1, help="Warmup runs per case")
    parser.add_argument(
        "--output", type=Path, help="Write Markdown results to this path"
    )
    return parser.parse_args()


def default_binary(profile: str) -> Path:
    return Path("target") / profile / "prism"


def main() -> int:
    args = parse_args()
    binary = args.bin or default_binary(args.build_profile)
    if args.build:
        build_binary(args.build_profile)
    if not binary.exists():
        print(
            f"missing binary: {binary}. Run with --build or pass --bin.",
            file=sys.stderr,
        )
        return 2
    if args.iterations < 1:
        print("--iterations must be at least 1", file=sys.stderr)
        return 2
    if args.warmups < 0:
        print("--warmups must be nonnegative", file=sys.stderr)
        return 2

    with tempfile.TemporaryDirectory(prefix="prism-perf-") as temp:
        cases = build_cases(Path(temp))
        groups = set(args.group or [])
        if args.startup_only:
            groups.add("startup")
        if groups:
            cases = [case for case in cases if case.group in groups]
        if not cases:
            print("no benchmark cases selected", file=sys.stderr)
            return 2
        results = [
            run_case(binary, case, args.warmups, args.iterations) for case in cases
        ]

    metadata = RunMetadata(
        binary=binary,
        binary_size_bytes=binary.stat().st_size,
        build_profile=args.build_profile,
        groups=tuple(sorted(groups)) if groups else ("all",),
        warmups=args.warmups,
        iterations=args.iterations,
    )
    markdown = render_markdown(results, metadata)
    if args.output:
        args.output.write_text(markdown, encoding="utf-8")
    else:
        print(markdown, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
