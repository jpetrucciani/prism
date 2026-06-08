#!/usr/bin/env python3
"""Smoke-test a built prism release binary."""

from __future__ import annotations

import argparse
import subprocess
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def run(
    binary: Path, *args: str, stdin: bytes = b""
) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        [str(binary), *args],
        input=stdin,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def expect(
    binary: Path,
    *args: str,
    stdin: bytes = b"",
    stdout: bytes | None = None,
    stderr_contains: bytes | None = None,
) -> None:
    result = run(binary, *args, stdin=stdin)
    if result.returncode != 0:
        raise AssertionError(
            f"{args} exited {result.returncode}; stderr={result.stderr.decode(errors='replace')}"
        )
    if stdout is not None and result.stdout != stdout:
        raise AssertionError(
            f"{args} stdout mismatch\nexpected={stdout!r}\nactual={result.stdout!r}"
        )
    if stderr_contains is not None and stderr_contains not in result.stderr:
        raise AssertionError(
            f"{args} stderr missing {stderr_contains!r}; stderr={result.stderr!r}"
        )


def package_version() -> str:
    with (ROOT / "Cargo.toml").open("rb") as file:
        return tomllib.load(file)["package"]["version"]


def main() -> int:
    parser = argparse.ArgumentParser(description="Smoke-test a built prism binary")
    parser.add_argument("--bin", type=Path, default=ROOT / "target/release/prism")
    args = parser.parse_args()

    binary = args.bin.resolve()
    if not binary.exists():
        print(f"missing binary: {binary}", file=sys.stderr)
        return 2

    version = package_version()
    expect(binary, "--version", stdout=f"prism {version}\n".encode())
    metadata = run(binary, "version")
    if metadata.returncode != 0:
        raise AssertionError(metadata.stderr.decode(errors="replace"))
    for required in [
        f"prism {version}".encode(),
        b"target:",
        b"build-profile:",
        b"build-commit:",
        b"rng-contract: prism-rng-v1",
        b"wordlist:",
    ]:
        if required not in metadata.stdout:
            raise AssertionError(f"version metadata missing {required!r}")

    expect(
        binary,
        "seq",
        "1..3",
        "--fmt",
        "item-%03d",
        stdout=b"item-001\nitem-002\nitem-003\n",
    )
    expect(binary, "case", "snake", "FooBar", stdout=b"foo_bar\n")
    expect(binary, "slug", "--sep", "_", "Hello, World!", stdout=b"hello_world\n")
    expect(
        binary, "field", "2..-1", "--osep", ",", stdin=b"a b c d\n", stdout=b"b,c,d\n"
    )
    expect(binary, "enc", "hex", stdin=b"abc", stdout=b"616263\n")
    expect(
        binary,
        "hash",
        "sha256",
        stdin=b"abc",
        stdout=b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n",
    )
    expect(
        binary,
        "do",
        "trim | case snake | slug",
        stdin=b"  Hello World  \n",
        stdout=b"hello-world\n",
    )
    expect(binary, "--json", "field", "2", stdin=b"a b\n", stdout=b'"b"\n')
    expect(binary, "-0", "case", "upper", stdin=b"a\0b\0", stdout=b"A\0B\0")

    first = run(binary, "--seed", "release-smoke", "rand", "--hex", "16")
    second = run(binary, "--seed", "release-smoke", "rand", "--hex", "16")
    if first.returncode != 0 or second.returncode != 0 or first.stdout != second.stdout:
        raise AssertionError("seeded rand is not deterministic across two runs")

    completions = run(binary, "completions", "bash")
    if completions.returncode != 0:
        raise AssertionError(completions.stderr.decode(errors="replace"))
    if b"prism" not in completions.stdout or b"completions" not in completions.stdout:
        raise AssertionError(
            "bash completions output does not mention prism/completions"
        )

    print(f"release smoke passed: {binary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
