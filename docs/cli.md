# prism CLI reference

This is the user-facing reference for the v1 command surface. `SPEC.md` remains the normative behavior document; this file is the shorter operator reference.

## Global flags

| Flag                | Meaning                                                                            |
| ------------------- | ---------------------------------------------------------------------------------- |
| `-i, --in <FILE>`   | Read input from file instead of stdin. `-` means stdin.                            |
| `-o, --out <FILE>`  | Write output to file instead of stdout.                                            |
| `--append`          | Append to `--out` instead of atomic replacement.                                   |
| `--mkdir`           | Create missing parent directories for `--out`.                                     |
| `--mode <OCTAL>`    | Set output file mode.                                                              |
| `-n, --count <N>`   | Emit N records for generators. In chains, applies only to a first-stage generator. |
| `-s, --seed <SEED>` | Make randomness deterministic.                                                     |
| `-0, --null`        | Use NUL as input and output record separator.                                      |
| `--no-newline`      | Omit the final trailing record separator.                                          |
| `-r, --raw`         | Suppress verb-specific quoting or escaping.                                        |
| `--json`            | Render each output record as a JSON string literal.                                |
| `--keep-going`      | Continue per-record work after recoverable record errors.                          |
| `-q, --quiet`       | Suppress non-fatal diagnostics.                                                    |
| `-h, --help`        | Print help.                                                                        |
| `-V, --version`     | Print the conventional one-line Cargo package version.                             |

## Version and environment

```sh
prism --version
prism version
prism completions bash
```

`prism --version` prints only `prism <version>` for tooling compatibility. `prism version` prints extended metadata: Cargo package version, target, build profile, build commit when available, deterministic RNG contract, and builtin wordlist version.

`prism completions <shell>` prints shell completions to stdout. Supported shells include `bash`, `zsh`, `fish`, `powershell`, and `elvish`.

`prism` intentionally avoids broad ambient option overrides. There is no generic `PRISM_*` flag mapping. The supported environment inputs are:

| Variable            | Effect                                                                               |
| ------------------- | ------------------------------------------------------------------------------------ |
| `PRISM_TZ`          | Default timezone for `dt` when `--tz` and `--utc` are not set.                       |
| `TZ`                | Fallback default timezone for `dt` when `PRISM_TZ`, `--tz`, and `--utc` are not set. |
| `XDG_CONFIG_HOME`   | Moves the config path used for aliases and `[defaults]`.                             |
| Process environment | Read by `tpl` as template variables.                                                 |

Config `[defaults]` currently supports `seed`, `count`, `null`, `no_newline`, `raw`, `json`, `keep_going`, and `quiet`. Explicit CLI flags win over config defaults.

## Exit codes

| Code | Meaning                                                                           |
| ---- | --------------------------------------------------------------------------------- |
| `0`  | Success.                                                                          |
| `1`  | Runtime error, usually invalid input data or filesystem failure.                  |
| `2`  | Usage error, invalid flags, invalid command shape, or mutually exclusive options. |
| `3`  | Template variable error.                                                          |
| `4`  | Template recursion error.                                                         |

Fail-fast commands may have already written successful stdout records before a later record fails. Atomic `--out` writes roll back on fail-fast errors.

## Generation

### `dt`

```sh
prism dt
prism dt --iso
prism dt --epoch
prism dt --epoch-ms
prism dt --rfc3339
prism dt --rfc2822
prism dt --fmt '%Y-%m-%d'
prism dt --utc
prism dt --tz America/New_York
PRISM_TZ=America/New_York prism dt --fmt '%F %T %z'
prism dt -1d
prism dt +2h30m
prism dt --at 2025-01-01 +1mo
prism dt --from 1735689600 --fmt '%F'
```

Offsets use `+1y2mo3d4h5m6s` style components. Month and year arithmetic clamp to valid calendar days. Timezone precedence is `--utc`, then `--tz`, then `PRISM_TZ`, then `TZ`, then the local system timezone.

### `rand`

```sh
prism rand --hex 16
prism rand --alnum 32
prism rand --alpha 8
prism rand --digits 6
prism rand --base32 20
prism rand --base64 24
prism rand --ascii 12
prism rand --charset 'abc123' --len 10
prism rand --uuid
prism rand --uuid7
prism rand --ulid
prism rand --words 4 --sep '-'
prism rand --bytes 16 > bytes.bin
```

Use `--seed` for reproducible output. Seeded output is stable across supported platforms for the same prism major version, command, args, and seed.

### `seq`

```sh
prism seq a..z
prism seq aa..zz
prism seq 1..100 --pad 3
prism seq 1..10 --fmt 'item-%03d'
prism seq a..z --sep ','
prism seq 0..255 --hex
```

### `repeat`

```sh
prism repeat 'ab' 5
prism repeat '-' 40
prism repeat 'x' 3 --sep ', '
```

### `pad`

```sh
echo hi | prism pad --right 10
prism pad --left 6 --fill 0 42
echo hi | prism pad --center 10 --fill '*'
```

## Text transforms

### `case`

```sh
prism case snake FooBar
prism case camel foo_bar
prism case pascal foo_bar
prism case kebab FooBar
prism case scream foo_bar
prism case const foo_bar
prism case title 'foo bar'
prism case upper foo
prism case lower FOO
prism case swap Foo
prism case dot FooBar
prism case path FooBar
```

### `slug`

```sh
echo 'Hello, World!' | prism slug
prism slug --sep _ --max 40 'Some Title'
prism slug --unicode 'Creme Brulee Tokyo'
```

### `trim` and `squeeze`

```sh
prism trim
prism trim --left
prism trim --right
prism trim --chars '/'
prism squeeze
prism squeeze --char '/'
```

### `wrap`, `indent`, and `dedent`

```sh
prism wrap --width 72
prism wrap --width 72 --hanging 4
prism indent --spaces 4
prism indent --tabs 1
prism dedent
```

### `replace`

```sh
prism replace foo bar
prism replace --regex '\d+' 'N'
prism replace --regex '(\w+)@(\w+)' '$2.$1'
prism replace --first foo bar
```

### `field`

```sh
echo 'a b c' | prism field 2
echo 'a:b:c' | prism field 2 -d ':'
prism field 2,4 --osep ','
prism field 2..
prism field ..3
prism field 2..-1
prism field -1
```

Field specs are 1-based. Negative indices count from the end. Ranges use `..` and are inclusive.

### `slice`

```sh
echo hello | prism slice 0..3
echo hello | prism slice -3..
echo hello | prism slice 1
prism slice --bytes 0..4 'abcd'
prism slice --graphemes 0..1 'text'
```

## Lines

```sh
prism lines --number
prism lines --uniq
prism lines --uniq-global
prism lines --reverse
prism lines --shuffle --seed 42
prism lines --sort
prism lines --sort --numeric --reverse
```

## Encoding

```sh
prism enc base64
prism enc base64 -d
prism enc base64url
prism enc base64 --no-pad
prism enc base32
prism enc base32hex
prism enc hex --upper
prism enc url --component
prism enc html
prism enc xml
prism enc quoted-printable
prism enc ascii85
prism enc base85
prism enc rot13
prism enc punycode
prism enc shell
prism enc json
prism enc csv-field
prism enc hex --per-line
```

`enc` consumes the whole input by default. Use `--per-line` to operate per record.

## Hashing

```sh
prism hash sha256
prism hash sha512
prism hash md5
prism hash blake3
prism hash sha3-256
prism hash blake2b
prism hash crc32
prism hash xxh3
prism hash fnv1a
prism hash sha256 --upper
prism hash sha256 --raw
prism hash sha256 --base64
prism hash sha256 --short 12
prism hash hmac-sha256 --key secret
prism -i file.tar hash sha256
```

`hash` consumes the whole input by default. Use `--per-line` to hash each record.

## Templates

```sh
prism tpl --set PORT=8080 < template.txt
prism tpl --strict < template.txt
prism tpl --env-file .env < template.txt
prism tpl --recursive --max-depth 8 < template.txt
prism tpl --no-gen < untrusted-template.txt
```

Template forms:

```text
${VAR}
${VAR:-default}
${VAR:?message}
${VAR:+alt}
${@uuid}
${@now:%Y-%m-%d}
${@rand:hex:16}
${@slug:TEXT}
```

For untrusted templates, use `--no-gen`, keep recursion off, and run with a scrubbed environment.

## Quoting

```sh
prism quote shell
prism quote json
prism quote c
prism quote regex
prism quote sql
```

`quote json` and `enc json` produce the same JSON string literal. `--json` is for rendering another command's output as JSON.

## Chaining

```sh
echo '  Hello World  ' | prism do 'trim | case snake | slug'
prism do 'rand --hex 8 | case upper'
echo 'a b c' | prism do 'field 2 | case scream'
```

A generator may only be the first stage. Records flow natively between stages.

## Aliases

```sh
prism alias list
prism alias show snakeslug
prism alias add snakeslug 'trim | case snake | slug'
prism alias rm snakeslug
prism alias path

echo '  Hello World  ' | prism run snakeslug
prism x token
prism run branchname 'My New Feature!'
```

Config lives at `$XDG_CONFIG_HOME/prism/config.toml` or `~/.config/prism/config.toml`.
