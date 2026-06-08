use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::ffi::OsString;
use std::fmt::{self, Display};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE as B64_URL, URL_SAFE_NO_PAD};
use base64::Engine as _;
use blake2::{Blake2b512, Blake2s256};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell as CompletionShell};
use crc32fast::Hasher as Crc32;
use data_encoding::{BASE32, BASE32HEX, HEXLOWER, HEXUPPER};
use digest::Digest;
use hmac::{Hmac, Mac};
use percent_encoding::{percent_decode, utf8_percent_encode, AsciiSet, CONTROLS};
use rand::seq::SliceRandom;
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Sha224, Sha256, Sha384, Sha512};
use sha3::{Sha3_256, Sha3_512};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;
use xxhash_rust::xxh3::xxh3_64;
use xxhash_rust::xxh64::xxh64;

const RNG_CONTRACT: &str = "prism-rng-v1";
const WORDLIST_VERSION: &str = "builtin-demo-v1";
const URL_ENCODE: &AsciiSet = &CONTROLS.add(b' ');
const URL_COMPONENT_ENCODE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'\"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'=')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b']');

#[derive(Clone, Debug)]
struct PrismError {
    code: i32,
    message: String,
    partial_records: Option<Vec<String>>,
}

impl PrismError {
    fn runtime(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            message: message.into(),
            partial_records: None,
        }
    }

    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
            partial_records: None,
        }
    }

    fn template(message: impl Into<String>) -> Self {
        Self {
            code: 3,
            message: message.into(),
            partial_records: None,
        }
    }

    fn recursion(message: impl Into<String>) -> Self {
        Self {
            code: 4,
            message: message.into(),
            partial_records: None,
        }
    }
}

impl Display for PrismError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PrismError {}

impl From<io::Error> for PrismError {
    fn from(value: io::Error) -> Self {
        Self::runtime(value.to_string())
    }
}

type Result<T> = std::result::Result<T, PrismError>;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "prism",
    version = env!("CARGO_PKG_VERSION"),
    about = "Generate and transform text records from the command line",
    long_about = "prism is a Unix-style record pipeline for producing, transforming, encoding, hashing, and templating text.\n\nThe execution model is: acquire input, split into records, apply one verb, render each output record, then join records back together. Newline is the default record separator. Use -0/--null to switch both input splitting and output joining to NUL.\n\nGenerators such as dt, rand, seq, repeat, and pad with a positional value do not read stdin. Per-record transforms such as case, slug, trim, squeeze, replace, field, slice, and stdin-mode pad map over each record independently. Whole-stream transforms such as wrap, indent, dedent, lines, enc, hash, tpl, and quote consume the input as one unit unless the verb documents --per-line.\n\nEnvironment and defaults: prism keeps most behavior explicit and does not read broad PRISM_* option overrides. The dt verb uses PRISM_TZ, then TZ, as its default timezone when --tz and --utc are not set. XDG_CONFIG_HOME changes the config path used for aliases and defaults. Config [defaults] currently supports seed, count, null, no_newline, raw, json, keep_going, and quiet. The tpl verb reads the process environment as template variables, with --env-file and --set controlling precedence.\n\nExamples:\n  prism dt --utc --fmt '%Y-%m-%dT%H:%M:%SZ'\n  prism --seed demo rand --alnum 24\n  echo '  Hello World  ' | prism do 'trim | case snake | slug'\n  printf 'a\\nb\\n' | prism --json case upper\n\nUse --help on any verb for option-level details, for example: prism rand --help, prism enc --help, or prism tpl --help."
)]
#[command(
    args_conflicts_with_subcommands = false,
    subcommand_negates_reqs = true
)]
struct Cli {
    #[command(flatten)]
    global: Global,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Debug, Clone, Default)]
struct Global {
    /// Read input from PATH instead of stdin. Use '-' to force stdin.
    #[arg(short = 'i', long = "in", global = true)]
    input: Option<PathBuf>,
    /// Write final output to PATH instead of stdout. Non-append writes are atomic.
    #[arg(short = 'o', long = "out", global = true)]
    output: Option<PathBuf>,
    /// Append to --out instead of replacing it atomically.
    #[arg(long, global = true)]
    append: bool,
    /// Create missing parent directories for --out.
    #[arg(long, global = true)]
    mkdir: bool,
    /// Set output file mode as octal, for example 0644. Unix only.
    #[arg(long, id = "file-mode", global = true)]
    file_mode: Option<String>,
    /// Number of records to generate. Applies to generators, or the first stage of a chain.
    #[arg(short = 'n', long = "count", global = true)]
    count: Option<usize>,
    /// Seed deterministic random output. The seed is shared across a chain.
    #[arg(short = 's', long, global = true)]
    seed: Option<String>,
    /// Use NUL as both input record separator and output record separator.
    #[arg(short = '0', long = "null", global = true)]
    null: bool,
    /// Omit only the final record terminator.
    #[arg(long, global = true)]
    no_newline: bool,
    /// Suppress verb-specific quoting where the verb supports quoted output.
    #[arg(short = 'r', long, global = true)]
    raw: bool,
    /// Render each finished output record as a JSON string literal.
    #[arg(long, global = true)]
    json: bool,
    /// Continue after per-record failures, print successful records, and exit nonzero.
    #[arg(long, global = true)]
    keep_going: bool,
    /// Suppress nonessential diagnostics, including binary-mode rendering notes.
    #[arg(short = 'q', long, global = true)]
    quiet: bool,
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    /// Print package version plus deterministic-output metadata.
    #[command(after_help = "Examples:\n  prism version\n  prism --version")]
    Version,
    /// Generate or format datetimes.
    Dt(DtArgs),
    /// Generate random strings, identifiers, words, or bytes.
    Rand(RandArgs),
    /// Generate numeric or alphabetic sequences.
    Seq(SeqArgs),
    /// Repeat a value a fixed number of times.
    Repeat(RepeatArgs),
    /// Pad input records, or generate a padded positional value.
    Pad(PadArgs),
    /// Convert text case using deterministic Unicode-aware splitting.
    Case(CaseArgs),
    /// Convert text into URL/file-name friendly slugs.
    Slug(SlugArgs),
    /// Trim whitespace or selected characters from records.
    Trim(TrimArgs),
    /// Collapse repeated whitespace or repeated selected characters.
    Squeeze(SqueezeArgs),
    /// Reflow the whole input to a target display width.
    Wrap(WrapArgs),
    /// Prefix every line of the whole input.
    Indent(IndentArgs),
    /// Remove common leading indentation from the whole input.
    #[command(after_help = "Examples:\n  prism dedent\n  cat block.txt | prism dedent")]
    Dedent,
    /// Replace literal text or regex matches per record.
    Replace(ReplaceArgs),
    /// Select delimited fields from each record.
    Field(FieldArgs),
    /// Select character, byte, or grapheme ranges from each record.
    Slice(SliceArgs),
    /// Transform the complete record stream as lines.
    Lines(LinesArgs),
    /// Encode or decode bytes and text.
    Enc(EncArgs),
    /// Hash or HMAC the whole input, or each record with --per-line.
    Hash(HashArgs),
    /// Render shell-style templates with process environment variables and helpers.
    Tpl(TplArgs),
    /// Quote or escape the whole input for another syntax.
    Quote(QuoteArgs),
    /// Run a linear chain of prism stages.
    Do(DoArgs),
    /// Manage configured aliases.
    #[command(subcommand)]
    Alias(AliasCommand),
    /// Run a configured alias.
    Run(RunArgs),
    /// Short alias for 'run'.
    X(RunArgs),
    /// Generate shell completions.
    Completions(CompletionsArgs),
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism dt --utc --fmt '%Y-%m-%dT%H:%M:%SZ'\n  prism dt --tz America/New_York --at 2026-01-31 +1mo\n  PRISM_TZ=America/New_York prism dt --fmt '%F %T %z'"
)]
struct DtArgs {
    /// Emit RFC 3339 output. This is also the default format.
    #[arg(long)]
    iso: bool,
    /// Emit POSIX seconds since the Unix epoch.
    #[arg(long)]
    epoch: bool,
    /// Emit POSIX milliseconds since the Unix epoch.
    #[arg(long = "epoch-ms")]
    epoch_ms: bool,
    /// Emit RFC 3339 output. Alias for --iso.
    #[arg(long)]
    rfc3339: bool,
    /// Emit RFC 2822 output.
    #[arg(long)]
    rfc2822: bool,
    /// Format with a chrono strftime pattern, for example '%Y-%m-%d'.
    #[arg(long)]
    fmt: Option<String>,
    /// Format in UTC.
    #[arg(long)]
    utc: bool,
    /// Format in an IANA timezone, for example America/New_York. Defaults to PRISM_TZ, then TZ, then the local timezone.
    #[arg(long)]
    tz: Option<String>,
    /// Anchor the datetime before applying OFFSET. Accepts RFC3339, YYYY-MM-DD, or YYYY-MM-DDTHH:MM:SS.
    #[arg(long)]
    at: Option<String>,
    /// Anchor from POSIX seconds.
    #[arg(long = "from")]
    from_ts: Option<i64>,
    /// Use a fixed timestamp instead of the current clock.
    #[arg(long)]
    now: Option<String>,
    /// Offset to apply after anchoring, for example +1d, -2h, or +1mo.
    offset: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism completions bash > prism.bash\n  prism completions zsh > _prism\n  prism completions fish > prism.fish"
)]
struct CompletionsArgs {
    /// Shell to generate completions for.
    #[arg(value_enum)]
    shell: CompletionShell,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism rand --hex 16\n  prism --seed demo rand --alnum 32\n  prism rand --uuid7\n  prism rand --bytes 16 > bytes.bin"
)]
struct RandArgs {
    /// Generate LEN random bytes and encode them as lowercase hex.
    #[arg(long)]
    hex: Option<usize>,
    /// Generate LEN random ASCII letters or digits.
    #[arg(long)]
    alnum: Option<usize>,
    /// Generate LEN random ASCII letters.
    #[arg(long)]
    alpha: Option<usize>,
    /// Generate LEN random decimal digits.
    #[arg(long)]
    digits: Option<usize>,
    /// Generate LEN random bytes and encode them as base32.
    #[arg(long)]
    base32: Option<usize>,
    /// Generate LEN random bytes and encode them as base64.
    #[arg(long)]
    base64: Option<usize>,
    /// Generate LEN random printable ASCII characters.
    #[arg(long)]
    ascii: Option<usize>,
    /// Generate from this exact character set. Requires --len.
    #[arg(long)]
    charset: Option<String>,
    /// Length used with --charset.
    #[arg(long)]
    len: Option<usize>,
    /// Generate an RFC 4122 version 4 UUID.
    #[arg(long)]
    uuid: bool,
    /// Generate a UUIDv7. With --seed, the timestamp is deterministic unless --now is set.
    #[arg(long)]
    uuid7: bool,
    /// Generate a ULID. With --seed, the timestamp is deterministic unless --now is set.
    #[arg(long)]
    ulid: bool,
    /// Generate COUNT words from the builtin or supplied wordlist.
    #[arg(long)]
    words: Option<usize>,
    /// Separator used between generated words.
    #[arg(long, default_value = " ")]
    sep: String,
    /// Read words from PATH instead of the builtin wordlist.
    #[arg(long)]
    wordlist: Option<PathBuf>,
    /// Emit LEN raw random bytes. This binary mode bypasses record framing.
    #[arg(long)]
    bytes: Option<usize>,
    /// Fixed timestamp in milliseconds for uuid7 or ulid.
    #[arg(long)]
    now: Option<u64>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism seq 1..5\n  prism seq 1..100 --pad 3\n  prism seq 1..10 --fmt 'item-%03d'\n  prism seq a..z --sep ,"
)]
struct SeqArgs {
    /// Inclusive range, for example 1..5, 5..1, or a..z.
    range: String,
    /// Zero-pad generated numbers to WIDTH.
    #[arg(long)]
    pad: Option<usize>,
    /// Format generated numbers with a simple %d pattern.
    #[arg(long)]
    fmt: Option<String>,
    /// Join the generated sequence into one record with SEP.
    #[arg(long)]
    sep: Option<String>,
    /// Format generated numbers as lowercase hex.
    #[arg(long)]
    hex: bool,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism repeat '-' 40\n  prism repeat ab 5\n  prism repeat x 3 --sep ', '"
)]
struct RepeatArgs {
    /// Text to repeat.
    value: String,
    /// Number of repetitions.
    count: usize,
    /// Separator inserted between repetitions.
    #[arg(long)]
    sep: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  echo hi | prism pad --right 10\n  prism pad --left 6 --fill 0 42\n  echo hi | prism pad --center 10 --fill '*'"
)]
struct PadArgs {
    /// Pad on the left until the record reaches WIDTH.
    #[arg(long)]
    left: Option<usize>,
    /// Pad on the right until the record reaches WIDTH.
    #[arg(long)]
    right: Option<usize>,
    /// Pad on both sides until the record reaches WIDTH.
    #[arg(long)]
    center: Option<usize>,
    /// Fill string to repeat while padding.
    #[arg(long, default_value = " ")]
    fill: String,
    /// Width accounting mode: chars, bytes, or display.
    #[arg(long, default_value = "display")]
    width_mode: WidthMode,
    /// Optional value to pad without reading stdin.
    value: Option<String>,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
enum WidthMode {
    Chars,
    Bytes,
    Display,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism case snake FooBar\n  prism case camel foo_bar\n  prism case upper foo\n  echo 'Hello World' | prism case kebab"
)]
struct CaseArgs {
    /// Case mode: snake, camel, pascal, kebab, scream, const, title, upper, lower, swap, dot, or path.
    mode: CaseMode,
    /// Optional value to transform without reading stdin.
    value: Option<String>,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
enum CaseMode {
    Snake,
    Camel,
    Pascal,
    Kebab,
    Scream,
    Const,
    Title,
    Upper,
    Lower,
    Swap,
    Dot,
    Path,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  echo 'Hello, World!' | prism slug\n  prism slug --sep _ --max 40 'Some Title'\n  prism slug --unicode 'Creme Brulee Tokyo'"
)]
struct SlugArgs {
    /// Separator to insert between slug words.
    #[arg(long, default_value = "-")]
    sep: String,
    /// Maximum output length in Unicode scalar values.
    #[arg(long)]
    max: Option<usize>,
    /// Keep lowercased non-ASCII letters instead of dropping them.
    #[arg(long)]
    unicode: bool,
    /// Optional value to slugify without reading stdin.
    value: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism trim\n  prism trim --left\n  prism trim --right\n  prism trim --chars /"
)]
struct TrimArgs {
    /// Trim only from the left side.
    #[arg(long)]
    left: bool,
    /// Trim only from the right side.
    #[arg(long)]
    right: bool,
    /// Trim these exact characters instead of whitespace.
    #[arg(long)]
    chars: Option<String>,
    /// Treat only ASCII whitespace as whitespace.
    #[arg(long)]
    ascii: bool,
    /// Optional value to trim without reading stdin.
    value: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism squeeze\n  prism squeeze --ascii\n  prism squeeze --char /"
)]
struct SqueezeArgs {
    /// Collapse runs of this exact string instead of whitespace.
    #[arg(long)]
    char: Option<String>,
    /// Treat only ASCII whitespace as whitespace.
    #[arg(long)]
    ascii: bool,
    /// Optional value to squeeze without reading stdin.
    value: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism wrap --width 72\n  prism wrap --width 72 --hanging 4\n  prism wrap --width 40 'short text'"
)]
struct WrapArgs {
    /// Target line width.
    #[arg(long)]
    width: usize,
    /// Number of spaces to prefix continuation lines with.
    #[arg(long, default_value_t = 0)]
    hanging: usize,
    /// Width accounting mode: chars, bytes, or display.
    #[arg(long, default_value = "display")]
    width_mode: WidthMode,
    /// Optional value to wrap without reading stdin.
    value: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism indent --spaces 4\n  prism indent --tabs 1\n  prism indent --spaces 2 'line'"
)]
struct IndentArgs {
    /// Spaces to prepend to every line.
    #[arg(long, default_value_t = 0)]
    spaces: usize,
    /// Tabs to prepend before spaces on every line.
    #[arg(long, default_value_t = 0)]
    tabs: usize,
    /// Optional value to indent without reading stdin.
    value: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism replace foo bar\n  prism replace --regex '\\d+' N\n  prism replace --first foo bar"
)]
struct ReplaceArgs {
    /// Treat NEEDLE as a regular expression.
    #[arg(long)]
    regex: bool,
    /// Replace only the first match in each record.
    #[arg(long)]
    first: bool,
    /// Literal text or regex pattern to find.
    needle: String,
    /// Replacement text.
    replacement: String,
    /// Optional value to replace within without reading stdin.
    value: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  echo 'a b c' | prism field 2\n  echo 'a:b:c' | prism field 2 -d :\n  prism field 2..-1 --osep ,"
)]
struct FieldArgs {
    /// Field selection, for example 2, -1, 2..4, 2.., ..3, or 2,4.
    spec: String,
    /// Literal delimiter. Defaults to runs of whitespace.
    #[arg(short = 'd', long = "delimiter")]
    delimiter: Option<String>,
    /// Treat --delimiter as a regular expression.
    #[arg(long)]
    regex: bool,
    /// Output separator between selected fields.
    #[arg(long, default_value = " ")]
    osep: String,
    /// Error when an index or range is out of bounds instead of emitting blanks or nothing.
    #[arg(long = "strict-fields")]
    strict_fields: bool,
    /// Optional value to split without reading stdin.
    value: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  echo hello | prism slice 0..3\n  echo hello | prism slice -3..\n  prism slice --bytes 0..4 abcd\n  prism slice --graphemes 0..1 'text'"
)]
struct SliceArgs {
    /// Slice spec, for example 0, -1, 1..4, 1.., or ..5.
    spec: String,
    /// Slice by bytes. Ranges must land on valid UTF-8 boundaries.
    #[arg(long)]
    bytes: bool,
    /// Slice by Unicode grapheme clusters instead of scalar values.
    #[arg(long)]
    graphemes: bool,
    /// Optional value to slice without reading stdin.
    value: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism lines --number\n  prism lines --uniq-global\n  prism lines --sort --numeric\n  prism --seed demo lines --shuffle"
)]
struct LinesArgs {
    /// Prefix each record with a one-based line number and a tab.
    #[arg(long)]
    number: bool,
    /// Remove adjacent duplicate records.
    #[arg(long)]
    uniq: bool,
    /// Remove duplicate records across the whole stream.
    #[arg(long = "uniq-global")]
    uniq_global: bool,
    /// Reverse record order.
    #[arg(long)]
    reverse: bool,
    /// Shuffle records. Respects --seed for deterministic output.
    #[arg(long)]
    shuffle: bool,
    /// Sort records lexicographically.
    #[arg(long)]
    sort: bool,
    /// Sort numerically. Only meaningful with --sort.
    #[arg(long)]
    numeric: bool,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism enc base64\n  prism enc base64 -d\n  prism enc hex --upper\n  prism enc url --component\n  prism enc hex --per-line"
)]
struct EncArgs {
    /// Codec: base64, base64url, base32, base32hex, hex, base16, url, html, xml, quoted-printable, ascii85, base85, rot13, punycode, shell, json, or csv-field.
    codec: String,
    /// Decode instead of encode.
    #[arg(short = 'd', long)]
    decode: bool,
    /// Omit base64 padding where the codec supports it.
    #[arg(long = "no-pad")]
    no_pad: bool,
    /// Use uppercase output for hex-like codecs.
    #[arg(long)]
    upper: bool,
    /// Percent-encode as a URL component instead of a broader URL string.
    #[arg(long)]
    component: bool,
    /// Encode or decode each record independently instead of the whole input.
    #[arg(short = 'l', long = "per-line")]
    per_line: bool,
    /// Optional value to encode or decode without reading stdin.
    value: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism hash sha256\n  prism hash blake3\n  prism hash sha256 --per-line\n  prism hash hmac-sha256 --key secret"
)]
struct HashArgs {
    /// Algorithm: md5, sha1, sha224, sha256, sha384, sha512, sha3-256, sha3-512, blake2b, blake2s, blake3, crc32, xxh64, xxh3, fnv1a, or hmac-*.
    algorithm: String,
    /// Render hexadecimal output in uppercase.
    #[arg(long)]
    upper: bool,
    /// Emit raw digest bytes. This binary mode bypasses record framing.
    #[arg(long)]
    raw: bool,
    /// Render the digest as base64 instead of hex.
    #[arg(long)]
    base64: bool,
    /// Truncate rendered output to N characters.
    #[arg(long)]
    short: Option<usize>,
    /// HMAC key. Required when ALGORITHM starts with hmac-.
    #[arg(long)]
    key: Option<String>,
    /// Hash each record independently instead of the whole input.
    #[arg(short = 'l', long = "per-line")]
    per_line: bool,
    /// Optional value to hash without reading stdin.
    value: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism tpl --set PORT=8080 'port=${PORT}'\n  prism tpl --env-file .env < template.txt\n  prism tpl --no-gen '${NAME:-unknown}'"
)]
struct TplArgs {
    /// Error when a variable without a default is missing or empty.
    #[arg(long)]
    strict: bool,
    /// Re-expand rendered output until stable, capped by --max-depth.
    #[arg(long)]
    recursive: bool,
    /// Maximum recursive expansion depth.
    #[arg(long, default_value_t = 16)]
    max_depth: usize,
    /// Read dotenv KEY=VALUE entries from PATH. Process environment values win unless --env-file-override is set.
    #[arg(long = "env-file")]
    env_file: Option<PathBuf>,
    /// Let --env-file values override process environment values.
    #[arg(long = "env-file-override")]
    env_file_override: bool,
    /// Set or override a template variable. May be repeated and always wins over env and --env-file.
    #[arg(long = "set")]
    set: Vec<String>,
    /// Disable @ generator helpers for hardened template processing.
    #[arg(long = "no-gen")]
    no_gen: bool,
    /// Optional template text to render without reading stdin.
    value: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism quote shell 'hello world'\n  prism quote json 'hello world'\n  prism quote regex 'a+b?'"
)]
struct QuoteArgs {
    /// Quote syntax: shell, json, c, regex, or sql.
    kind: QuoteKind,
    /// Optional value to quote without reading stdin.
    value: Option<String>,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
enum QuoteKind {
    Shell,
    Json,
    C,
    Regex,
    Sql,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  echo '  Hello World  ' | prism do 'trim | case snake | slug'\n  prism do 'seq 1..3 | pad --left 3 --fill 0'"
)]
struct DoArgs {
    /// Pipeline stages separated by '|', for example 'trim | case snake | slug'.
    chain: String,
}

#[derive(Subcommand, Debug, Clone)]
enum AliasCommand {
    /// List configured alias names.
    #[command(after_help = "Examples:\n  prism alias list")]
    List,
    /// Print the chain for one alias.
    #[command(after_help = "Examples:\n  prism alias show branchname")]
    Show {
        /// Alias name.
        name: String,
    },
    /// Add or replace an alias.
    #[command(after_help = "Examples:\n  prism alias add branchname 'trim | case snake | slug'")]
    Add {
        /// Alias name.
        name: String,
        /// Chain to store for this alias.
        chain: String,
    },
    /// Remove an alias.
    #[command(after_help = "Examples:\n  prism alias rm branchname")]
    Rm {
        /// Alias name.
        name: String,
    },
    /// Print the config file path.
    #[command(after_help = "Examples:\n  prism alias path")]
    Path,
}

#[derive(Args, Debug, Clone)]
#[command(
    after_help = "Examples:\n  prism run branchname 'My Feature'\n  echo 'My Feature' | prism run branchname\n  prism x branchname 'My Feature'"
)]
struct RunArgs {
    /// Alias name to run.
    alias: String,
    /// Positional arguments injected into the alias first stage.
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,
}

#[derive(Clone, Debug)]
enum Data {
    Records(Vec<String>),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
struct Config {
    #[serde(default)]
    alias: AliasConfig,
    #[serde(default)]
    defaults: BTreeMap<String, toml::Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
struct AliasConfig {
    #[serde(default)]
    options: AliasOptions,
    #[serde(flatten)]
    entries: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
struct AliasOptions {
    #[serde(default)]
    expand_bare: bool,
}

fn main() {
    let code = match run() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("prism: {err}");
            err.code
        }
    };
    process::exit(code);
}

fn run() -> Result<()> {
    let cli = match parse_cli() {
        Ok(cli) => cli,
        Err(err) if err.code == 0 => {
            print!("{}", err.message);
            return Ok(());
        }
        Err(err) => return Err(err),
    };
    let global = if matches!(cli.command, Command::Version | Command::Completions(_)) {
        cli.global
    } else {
        apply_config_defaults(cli.global)?
    };
    if global.raw && global.json {
        return Err(PrismError::usage("--raw and --json are mutually exclusive"));
    }
    if stream_output_fast_path(&cli.command, &global)? {
        return Ok(());
    }
    match execute_command(&cli.command, &global, None, false) {
        Ok(data) => write_output(data, &global),
        Err(mut err) => {
            if global.output.is_none() {
                if let Some(records) = err.partial_records.take() {
                    write_output(Data::Records(records), &global)?;
                }
            } else {
                err.partial_records = None;
            }
            Err(err)
        }
    }
}

fn version_info() -> String {
    format!(
        "prism {}\ntarget: {}-{}\nbuild-profile: {}\nbuild-commit: {}\nrng-contract: {}\nwordlist: {}",
        env!("CARGO_PKG_VERSION"),
        env::consts::ARCH,
        env::consts::OS,
        env!("PRISM_BUILD_PROFILE"),
        env!("PRISM_BUILD_COMMIT"),
        RNG_CONTRACT,
        WORDLIST_VERSION
    )
}

fn command_completions(args: &CompletionsArgs) -> Result<Data> {
    let mut command = Cli::command();
    let mut out = Vec::new();
    generate(args.shell, &mut command, "prism", &mut out);
    Ok(Data::Bytes(out))
}

fn apply_config_defaults(mut global: Global) -> Result<Global> {
    let config = load_config()?;
    if global.seed.is_none() {
        if let Some(seed) = config.defaults.get("seed").and_then(toml::Value::as_str) {
            global.seed = Some(seed.to_string());
        }
    }
    if global.count.is_none() {
        if let Some(count) = config
            .defaults
            .get("count")
            .and_then(toml::Value::as_integer)
        {
            global.count = Some(usize::try_from(count).map_err(|err| {
                PrismError::usage(format!("invalid defaults.count value: {err}"))
            })?);
        }
    }
    apply_bool_default(&mut global.null, &config, "null");
    apply_bool_default(&mut global.no_newline, &config, "no_newline");
    apply_bool_default(&mut global.raw, &config, "raw");
    apply_bool_default(&mut global.json, &config, "json");
    apply_bool_default(&mut global.keep_going, &config, "keep_going");
    apply_bool_default(&mut global.quiet, &config, "quiet");
    Ok(global)
}

fn apply_bool_default(target: &mut bool, config: &Config, key: &str) {
    if !*target {
        if let Some(value) = config.defaults.get(key).and_then(toml::Value::as_bool) {
            *target = value;
        }
    }
}

fn parse_cli() -> Result<Cli> {
    let mut args: Vec<OsString> = env::args_os().collect();
    if args.len() > 1 {
        if let Some(first) = args.get(1).and_then(|arg| arg.to_str()) {
            if is_potential_bare_alias(first) && !is_builtin(first) {
                if let Ok(config) = load_config() {
                    if config.alias.options.expand_bare && config.alias.entries.contains_key(first)
                    {
                        let alias = OsString::from(first);
                        args[1] = OsString::from("run");
                        args.insert(2, alias);
                    }
                }
            }
        }
    }
    Cli::try_parse_from(args).map_err(|err| PrismError {
        code: if err.use_stderr() { 2 } else { 0 },
        message: err.to_string(),
        partial_records: None,
    })
}

fn is_potential_bare_alias(value: &str) -> bool {
    !value.starts_with('-')
}

fn is_builtin(value: &str) -> bool {
    matches!(
        value,
        "dt" | "rand"
            | "seq"
            | "repeat"
            | "pad"
            | "case"
            | "slug"
            | "trim"
            | "squeeze"
            | "wrap"
            | "indent"
            | "dedent"
            | "replace"
            | "field"
            | "slice"
            | "lines"
            | "enc"
            | "hash"
            | "tpl"
            | "quote"
            | "do"
            | "alias"
            | "run"
            | "x"
            | "completions"
            | "version"
    )
}

fn execute_command(
    command: &Command,
    global: &Global,
    input: Option<Data>,
    in_chain: bool,
) -> Result<Data> {
    match command {
        Command::Version => Ok(Data::Records(vec![version_info()])),
        Command::Completions(args) => command_completions(args),
        Command::Dt(args) => Ok(Data::Records(command_dt(args, global)?)),
        Command::Rand(args) => command_rand(args, global),
        Command::Seq(args) => Ok(Data::Records(command_seq(args)?)),
        Command::Repeat(args) => Ok(Data::Records(vec![command_repeat(args)])),
        Command::Pad(args) => command_pad(args, global, input),
        Command::Case(args) => map_records(global, input, args.value.as_deref(), |record| {
            Ok(convert_case(record, args.mode))
        }),
        Command::Slug(args) => map_records(global, input, args.value.as_deref(), |record| {
            Ok(slugify(record, &args.sep, args.max, args.unicode))
        }),
        Command::Trim(args) => map_records(global, input, args.value.as_deref(), |record| {
            Ok(trim_record(record, args))
        }),
        Command::Squeeze(args) => {
            let repeated_char = compile_squeeze_regex(args)?;
            map_records(global, input, args.value.as_deref(), |record| {
                Ok(squeeze_record(record, args, repeated_char.as_ref()))
            })
        }
        Command::Wrap(args) => {
            let text = whole_text(global, input, args.value.as_deref())?;
            Ok(Data::Records(vec![wrap_text(
                &text,
                args.width,
                args.hanging,
                args.width_mode,
            )]))
        }
        Command::Indent(args) => {
            let text = whole_text(global, input, args.value.as_deref())?;
            Ok(Data::Records(vec![indent_text(
                &text,
                args.spaces,
                args.tabs,
            )]))
        }
        Command::Dedent => {
            let text = whole_text(global, input, None)?;
            Ok(Data::Records(vec![dedent_text(&text)]))
        }
        Command::Replace(args) => {
            if args.regex {
                let regex = Regex::new(&args.needle)
                    .map_err(|err| PrismError::usage(format!("invalid regex: {err}")))?;
                map_records(global, input, args.value.as_deref(), |record| {
                    Ok(replace_regex_record(record, args, &regex))
                })
            } else {
                map_records(global, input, args.value.as_deref(), |record| {
                    Ok(replace_literal_record(record, args))
                })
            }
        }
        Command::Field(args) => {
            let selections = parse_field_spec(&args.spec)?;
            let osep = decode_escapes(&args.osep);
            let delimiter_regex = compile_field_delimiter_regex(args)?;
            map_records(global, input, args.value.as_deref(), |record| {
                field_record(record, args, &selections, &osep, delimiter_regex.as_ref())
            })
        }
        Command::Slice(args) => {
            let selection = parse_slice_spec(&args.spec)?;
            map_records(global, input, args.value.as_deref(), |record| {
                slice_record(record, args, selection)
            })
        }
        Command::Lines(args) => {
            let records = records_from_input(global, input, None)?;
            Ok(Data::Records(lines_records(records, args, global)?))
        }
        Command::Enc(args) => command_enc(args, global, input),
        Command::Hash(args) => command_hash(args, global, input),
        Command::Tpl(args) => {
            let text = whole_text(global, input, args.value.as_deref())?;
            Ok(Data::Records(vec![render_template(&text, args, global)?]))
        }
        Command::Quote(args) => {
            let text = whole_text(global, input, args.value.as_deref())?;
            Ok(Data::Records(vec![quote_value(&text, args.kind)?]))
        }
        Command::Do(args) => execute_chain(&args.chain, global, input),
        Command::Alias(args) => {
            if in_chain {
                return Err(PrismError::usage(
                    "alias management is not allowed inside chains",
                ));
            }
            Ok(Data::Records(vec![command_alias(args)?]))
        }
        Command::Run(args) | Command::X(args) => execute_alias(args, global, input),
    }
}

fn read_input_bytes(global: &Global) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    match &global.input {
        Some(path) if path.as_os_str() != "-" => {
            bytes = fs::read(path)?;
        }
        _ => {
            io::stdin().read_to_end(&mut bytes)?;
        }
    }
    Ok(bytes)
}

fn records_from_input(
    global: &Global,
    input: Option<Data>,
    positional: Option<&str>,
) -> Result<Vec<String>> {
    if let Some(value) = positional {
        return Ok(vec![value.to_string()]);
    }
    match input {
        Some(Data::Records(records)) => Ok(records),
        Some(Data::Bytes(bytes)) => split_records(bytes, global.null),
        None => split_records(read_input_bytes(global)?, global.null),
    }
}

fn whole_text(global: &Global, input: Option<Data>, positional: Option<&str>) -> Result<String> {
    if let Some(value) = positional {
        return Ok(value.to_string());
    }
    match input {
        Some(Data::Records(records)) => Ok(records.join(if global.null { "\0" } else { "\n" })),
        Some(Data::Bytes(bytes)) => String::from_utf8(bytes)
            .map_err(|err| PrismError::runtime(format!("invalid UTF-8 input: {err}"))),
        None => String::from_utf8(read_input_bytes(global)?)
            .map_err(|err| PrismError::runtime(format!("invalid UTF-8 input: {err}"))),
    }
}

fn input_bytes(global: &Global, input: Option<Data>, positional: Option<&str>) -> Result<Vec<u8>> {
    if let Some(value) = positional {
        return Ok(value.as_bytes().to_vec());
    }
    match input {
        Some(Data::Records(records)) => Ok(records
            .join(if global.null { "\0" } else { "\n" })
            .into_bytes()),
        Some(Data::Bytes(bytes)) => Ok(bytes),
        None => read_input_bytes(global),
    }
}

fn stream_output_fast_path(command: &Command, global: &Global) -> Result<bool> {
    if let Some(path) = &global.output {
        if global.append {
            return Ok(false);
        }
        return stream_records_to_file_fast_path(command, global, path);
    }
    stream_stdout_fast_path(command, global)
}

fn stream_stdout_fast_path(command: &Command, global: &Global) -> Result<bool> {
    let Some(mut transform) = record_transform(command)? else {
        return Ok(false);
    };
    let stdout = io::stdout();
    let mut stdout = io::BufWriter::new(stdout.lock());
    stream_records_to_writer(global, &mut stdout, |record| transform.apply(record))?;
    stdout.flush()?;
    Ok(true)
}

fn stream_records_to_file_fast_path(
    command: &Command,
    global: &Global,
    path: &Path,
) -> Result<bool> {
    let Some(mut transform) = record_transform(command)? else {
        return Ok(false);
    };
    write_atomic_file(path, global, |writer| {
        stream_records_to_writer(global, writer, |record| transform.apply(record))
    })?;
    Ok(true)
}

fn stream_records_to_writer<W, F>(global: &Global, writer: &mut W, mut transform: F) -> Result<()>
where
    W: Write,
    F: FnMut(&str) -> Result<String>,
{
    let mut bytes = read_input_bytes(global)?;
    let sep_byte = if global.null { 0 } else { b'\n' };
    if bytes.last().copied() == Some(sep_byte) {
        bytes.pop();
    }
    if bytes.is_empty() {
        return Ok(());
    }
    let text = String::from_utf8(bytes)
        .map_err(|err| PrismError::runtime(format!("invalid UTF-8 input: {err}")))?;
    let sep = if global.null { "\0" } else { "\n" };
    let split = if global.null { '\0' } else { '\n' };
    let mut emitted = false;
    let mut had_error = false;
    for (idx, record) in text.split(split).enumerate() {
        match transform(record) {
            Ok(value) => {
                if emitted {
                    writer.write_all(sep.as_bytes())?;
                }
                write_streamed_record(writer, &value, global.json)?;
                emitted = true;
            }
            Err(err) if global.keep_going => {
                had_error = true;
                if !global.quiet {
                    eprintln!("prism: record {}: {err}", idx + 1);
                }
            }
            Err(err) => return Err(err),
        }
    }
    if emitted && !global.no_newline {
        writer.write_all(sep.as_bytes())?;
    }
    if had_error {
        return Err(PrismError::runtime("one or more records failed"));
    }
    Ok(())
}

fn write_streamed_record<W: Write>(writer: &mut W, value: &str, json: bool) -> Result<()> {
    if json {
        serde_json::to_writer(&mut *writer, value)
            .map_err(|err| PrismError::runtime(format!("json render failed: {err}")))
    } else {
        writer.write_all(value.as_bytes()).map_err(PrismError::from)
    }
}

enum RecordTransform {
    Pad(PadArgs),
    Case(CaseMode),
    Slug {
        sep: String,
        max: Option<usize>,
        unicode: bool,
    },
    Trim(TrimArgs),
    Squeeze {
        args: SqueezeArgs,
        repeated_char: Option<Regex>,
    },
    ReplaceLiteral(ReplaceArgs),
    ReplaceRegex {
        args: ReplaceArgs,
        regex: Regex,
    },
    Field {
        args: FieldArgs,
        selections: Vec<FieldSelection>,
        osep: String,
        delimiter_regex: Option<Regex>,
    },
    Slice {
        args: SliceArgs,
        selection: SliceSelection,
    },
    Enc(EncArgs),
    Hash(HashArgs),
}

impl RecordTransform {
    fn apply(&mut self, record: &str) -> Result<String> {
        match self {
            Self::Pad(args) => Ok(pad_record(record, args)),
            Self::Case(mode) => Ok(convert_case(record, *mode)),
            Self::Slug { sep, max, unicode } => Ok(slugify(record, sep, *max, *unicode)),
            Self::Trim(args) => Ok(trim_record(record, args)),
            Self::Squeeze {
                args,
                repeated_char,
            } => Ok(squeeze_record(record, args, repeated_char.as_ref())),
            Self::ReplaceLiteral(args) => Ok(replace_literal_record(record, args)),
            Self::ReplaceRegex { args, regex } => Ok(replace_regex_record(record, args, regex)),
            Self::Field {
                args,
                selections,
                osep,
                delimiter_regex,
            } => field_record(record, args, selections, osep, delimiter_regex.as_ref()),
            Self::Slice { args, selection } => slice_record(record, args, *selection),
            Self::Enc(args) => enc_value(record.as_bytes(), args).and_then(|bytes| {
                String::from_utf8(bytes).map_err(|err| {
                    PrismError::runtime(format!("encoded value is not UTF-8: {err}"))
                })
            }),
            Self::Hash(args) => format_digest(
                hash_bytes(record.as_bytes(), &args.algorithm, args.key.as_deref())?,
                args,
            ),
        }
    }
}

fn record_transform(command: &Command) -> Result<Option<RecordTransform>> {
    match command {
        Command::Pad(args) if args.value.is_none() => Ok(Some(RecordTransform::Pad(args.clone()))),
        Command::Case(args) if args.value.is_none() => Ok(Some(RecordTransform::Case(args.mode))),
        Command::Slug(args) if args.value.is_none() => Ok(Some(RecordTransform::Slug {
            sep: args.sep.clone(),
            max: args.max,
            unicode: args.unicode,
        })),
        Command::Trim(args) if args.value.is_none() => {
            Ok(Some(RecordTransform::Trim(args.clone())))
        }
        Command::Squeeze(args) if args.value.is_none() => {
            let repeated_char = compile_squeeze_regex(args)?;
            Ok(Some(RecordTransform::Squeeze {
                args: args.clone(),
                repeated_char,
            }))
        }
        Command::Replace(args) if args.value.is_none() => {
            if args.regex {
                let regex = Regex::new(&args.needle)
                    .map_err(|err| PrismError::usage(format!("invalid regex: {err}")))?;
                Ok(Some(RecordTransform::ReplaceRegex {
                    args: args.clone(),
                    regex,
                }))
            } else {
                Ok(Some(RecordTransform::ReplaceLiteral(args.clone())))
            }
        }
        Command::Field(args) if args.value.is_none() => {
            let selections = parse_field_spec(&args.spec)?;
            let osep = decode_escapes(&args.osep);
            let delimiter_regex = compile_field_delimiter_regex(args)?;
            Ok(Some(RecordTransform::Field {
                args: args.clone(),
                selections,
                osep,
                delimiter_regex,
            }))
        }
        Command::Slice(args) if args.value.is_none() => {
            let selection = parse_slice_spec(&args.spec)?;
            Ok(Some(RecordTransform::Slice {
                args: args.clone(),
                selection,
            }))
        }
        Command::Enc(args) if args.per_line && args.value.is_none() => {
            Ok(Some(RecordTransform::Enc(args.clone())))
        }
        Command::Hash(args) if args.per_line && !args.raw && args.value.is_none() => {
            Ok(Some(RecordTransform::Hash(args.clone())))
        }
        _ => Ok(None),
    }
}

fn split_records(mut bytes: Vec<u8>, null: bool) -> Result<Vec<String>> {
    let sep = if null { 0 } else { b'\n' };
    if bytes.last().copied() == Some(sep) {
        bytes.pop();
    }
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let text = String::from_utf8(bytes)
        .map_err(|err| PrismError::runtime(format!("invalid UTF-8 input: {err}")))?;
    let sep = if null { '\0' } else { '\n' };
    Ok(text.split(sep).map(ToString::to_string).collect())
}

fn map_records<F>(
    global: &Global,
    input: Option<Data>,
    positional: Option<&str>,
    mut f: F,
) -> Result<Data>
where
    F: FnMut(&str) -> Result<String>,
{
    let mut had_error = false;
    let mut out = Vec::new();
    for (idx, record) in records_from_input(global, input, positional)?
        .iter()
        .enumerate()
    {
        match f(record) {
            Ok(value) => out.push(value),
            Err(err) if global.keep_going => {
                had_error = true;
                if !global.quiet {
                    eprintln!("prism: record {}: {err}", idx + 1);
                }
            }
            Err(err) => return Err(err),
        }
    }
    if had_error {
        return Err(PrismError {
            code: 1,
            message: "one or more records failed".to_string(),
            partial_records: Some(out),
        });
    }
    Ok(Data::Records(out))
}

fn write_output(data: Data, global: &Global) -> Result<()> {
    let bytes = render_data(data, global)?;
    if let Some(path) = &global.output {
        write_file(path, &bytes, global)
    } else {
        io::stdout().write_all(&bytes)?;
        Ok(())
    }
}

fn render_data(data: Data, global: &Global) -> Result<Vec<u8>> {
    match data {
        Data::Bytes(bytes) => {
            if (global.json || global.no_newline) && !global.quiet {
                eprintln!("prism: binary output ignores --json and --no-newline");
            }
            Ok(bytes)
        }
        Data::Records(records) => {
            let rendered: Result<Vec<String>> = records
                .iter()
                .map(|record| {
                    if global.json {
                        serde_json::to_string(record).map_err(|err| {
                            PrismError::runtime(format!("json render failed: {err}"))
                        })
                    } else {
                        Ok(record.clone())
                    }
                })
                .collect();
            let rendered = rendered?;
            if rendered.is_empty() {
                return Ok(Vec::new());
            }
            let sep = if global.null { "\0" } else { "\n" };
            let mut output = rendered.join(sep).into_bytes();
            if !global.no_newline {
                output.extend_from_slice(sep.as_bytes());
            }
            Ok(output)
        }
    }
}

fn write_file(path: &Path, bytes: &[u8], global: &Global) -> Result<()> {
    ensure_output_parent(path, global)?;

    if global.append {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(bytes)?;
        file.flush()?;
        return Ok(());
    }

    write_atomic_file(path, global, |file| {
        file.write_all(bytes).map_err(PrismError::from)
    })
}

fn ensure_output_parent(path: &Path, global: &Global) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            if global.mkdir {
                fs::create_dir_all(parent)?;
            } else {
                return Err(PrismError::runtime(format!(
                    "parent directory does not exist: {}",
                    parent.display()
                )));
            }
        }
    }
    Ok(())
}

fn write_atomic_file<F>(path: &Path, global: &Global, mut write: F) -> Result<()>
where
    F: FnMut(&mut io::BufWriter<File>) -> Result<()>,
{
    ensure_output_parent(path, global)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| PrismError::runtime("output path has no valid file name"))?;
    let tmp = parent.join(format!(".{file_name}.tmp-{}", process::id()));
    let write_result = (|| -> Result<()> {
        let file = File::create(&tmp)?;
        let mut file = io::BufWriter::new(file);
        write(&mut file)?;
        file.flush()?;
        file.get_ref().sync_all()?;
        apply_mode(path, &tmp, global)?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result
}

#[cfg(unix)]
fn apply_mode(existing: &Path, tmp: &Path, global: &Global) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if let Some(mode) = &global.file_mode {
        u32::from_str_radix(mode.trim_start_matches('0'), 8)
            .map_err(|err| PrismError::usage(format!("invalid --mode: {err}")))?
    } else if existing.exists() {
        fs::metadata(existing)?.permissions().mode() & 0o7777
    } else {
        return Ok(());
    };
    let mut perms = fs::metadata(tmp)?.permissions();
    perms.set_mode(mode);
    fs::set_permissions(tmp, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn apply_mode(_existing: &Path, _tmp: &Path, _global: &Global) -> Result<()> {
    Ok(())
}

fn command_dt(args: &DtArgs, global: &Global) -> Result<Vec<String>> {
    let count = global.count.unwrap_or(1);
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let dt = base_datetime(args)?;
        let shifted = if let Some(offset) = &args.offset {
            apply_offset(dt, offset)?
        } else {
            dt
        };
        out.push(format_datetime(shifted, args));
    }
    Ok(out)
}

fn base_datetime(args: &DtArgs) -> Result<DateTime<chrono::FixedOffset>> {
    if let Some(from_ts) = args.from_ts {
        let utc = Utc
            .timestamp_opt(from_ts, 0)
            .single()
            .ok_or_else(|| PrismError::runtime("invalid --from timestamp"))?;
        return convert_timezone(utc, args);
    }
    if let Some(at) = args.at.as_ref().or(args.now.as_ref()) {
        return parse_datetime(at, args);
    }
    convert_timezone(Utc::now(), args)
}

fn convert_timezone(utc: DateTime<Utc>, args: &DtArgs) -> Result<DateTime<chrono::FixedOffset>> {
    if args.utc {
        return Ok(utc.fixed_offset());
    }
    if let Some(tz) = configured_timezone(args)? {
        return Ok(utc.with_timezone(&tz).fixed_offset());
    }
    Ok(utc.with_timezone(&Local).fixed_offset())
}

fn configured_timezone(args: &DtArgs) -> Result<Option<Tz>> {
    let source = args
        .tz
        .as_ref()
        .map(|value| ("--tz".to_string(), value.clone()))
        .or_else(|| env_timezone("PRISM_TZ"))
        .or_else(|| env_timezone("TZ"));
    let Some((label, name)) = source else {
        return Ok(None);
    };
    Tz::from_str(&name)
        .map(Some)
        .map_err(|err| PrismError::usage(format!("invalid timezone from {label}={name}: {err}")))
}

fn env_timezone(name: &str) -> Option<(String, String)> {
    let value = env::var(name).ok()?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some((name.to_string(), value.to_string()))
}

fn parse_datetime(value: &str, args: &DtArgs) -> Result<DateTime<chrono::FixedOffset>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Ok(dt);
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        let naive = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| PrismError::runtime("invalid date anchor"))?;
        return localize_naive(naive, args);
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        return localize_naive(naive, args);
    }
    Err(PrismError::runtime(format!(
        "could not parse datetime: {value}"
    )))
}

fn localize_naive(naive: NaiveDateTime, args: &DtArgs) -> Result<DateTime<chrono::FixedOffset>> {
    if args.utc {
        return Ok(Utc.from_utc_datetime(&naive).fixed_offset());
    }
    if let Some(tz) = configured_timezone(args)? {
        return match tz.from_local_datetime(&naive) {
            chrono::LocalResult::Single(dt) => Ok(dt.fixed_offset()),
            chrono::LocalResult::Ambiguous(earlier, _) => Ok(earlier.fixed_offset()),
            chrono::LocalResult::None => {
                let shifted = naive + Duration::hours(1);
                tz.from_local_datetime(&shifted)
                    .single()
                    .map(|dt| dt.fixed_offset())
                    .ok_or_else(|| PrismError::runtime("invalid local time"))
            }
        };
    }
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => Ok(dt.fixed_offset()),
        chrono::LocalResult::Ambiguous(earlier, _) => Ok(earlier.fixed_offset()),
        chrono::LocalResult::None => Ok(Local.from_utc_datetime(&naive).fixed_offset()),
    }
}

fn apply_offset(
    mut dt: DateTime<chrono::FixedOffset>,
    offset: &str,
) -> Result<DateTime<chrono::FixedOffset>> {
    let bytes = offset.as_bytes();
    let sign = match bytes.first().copied() {
        Some(b'+') => 1,
        Some(b'-') => -1,
        _ => return Err(PrismError::usage("offset must start with + or -")),
    };
    let mut index = 1;
    while index < bytes.len() {
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if start == index {
            return Err(PrismError::usage("offset component missing number"));
        }
        let number: i32 = offset[start..index]
            .parse()
            .map_err(|err| PrismError::usage(format!("invalid offset number: {err}")))?;
        let unit_start = index;
        while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
            index += 1;
        }
        let unit = &offset[unit_start..index];
        let amount = number * sign;
        dt = match unit {
            "y" => add_months(dt, amount * 12)?,
            "mo" => add_months(dt, amount)?,
            "w" => dt + Duration::weeks(i64::from(amount)),
            "d" => dt + Duration::days(i64::from(amount)),
            "h" => dt + Duration::hours(i64::from(amount)),
            "m" => dt + Duration::minutes(i64::from(amount)),
            "s" => dt + Duration::seconds(i64::from(amount)),
            _ => return Err(PrismError::usage(format!("invalid offset unit: {unit}"))),
        };
    }
    Ok(dt)
}

fn add_months(
    dt: DateTime<chrono::FixedOffset>,
    months: i32,
) -> Result<DateTime<chrono::FixedOffset>> {
    let naive = dt.naive_local();
    let total = naive.year() * 12 + i32::from(naive.month() as u16) - 1 + months;
    let year = total.div_euclid(12);
    let month = (total.rem_euclid(12) + 1) as u32;
    let day = naive.day().min(last_day_of_month(year, month)?);
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| PrismError::runtime("month arithmetic produced invalid date"))?;
    let shifted = date.and_time(naive.time());
    dt.timezone()
        .from_local_datetime(&shifted)
        .single()
        .ok_or_else(|| PrismError::runtime("month arithmetic produced invalid local time"))
}

fn last_day_of_month(year: i32, month: u32) -> Result<u32> {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .ok_or_else(|| PrismError::runtime("invalid month"))?;
    Ok((first_next - Duration::days(1)).day())
}

fn format_datetime(dt: DateTime<chrono::FixedOffset>, args: &DtArgs) -> String {
    if args.epoch {
        return dt.timestamp().to_string();
    }
    if args.epoch_ms {
        return dt.timestamp_millis().to_string();
    }
    if args.rfc2822 {
        return dt.to_rfc2822();
    }
    if let Some(fmt) = &args.fmt {
        return dt.format(fmt).to_string();
    }
    if args.iso || args.rfc3339 {
        return dt.to_rfc3339();
    }
    dt.to_rfc3339()
}

fn command_rand(args: &RandArgs, global: &Global) -> Result<Data> {
    if let Some(len) = args.bytes {
        let mut rng = rng_from_global(global, "rand --bytes");
        let mut bytes = vec![0_u8; len];
        rng.fill_bytes(&mut bytes);
        return Ok(Data::Bytes(bytes));
    }
    let count = global.count.unwrap_or(1);
    let mut rng = rng_from_global(global, "rand");
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(rand_text(args, global, &mut rng)?);
    }
    Ok(Data::Records(out))
}

fn rng_from_global(global: &Global, stream: &str) -> ChaCha20Rng {
    let seed = global.seed.as_deref().unwrap_or("entropy");
    let mut hasher = blake3::Hasher::new();
    hasher.update(RNG_CONTRACT.as_bytes());
    hasher.update(WORDLIST_VERSION.as_bytes());
    hasher.update(stream.as_bytes());
    hasher.update(seed.as_bytes());
    if global.seed.is_none() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        hasher.update(&now.to_le_bytes());
    }
    let digest = hasher.finalize();
    let mut key = [0_u8; 32];
    key.copy_from_slice(digest.as_bytes());
    ChaCha20Rng::from_seed(key)
}

fn rand_text(args: &RandArgs, global: &Global, rng: &mut ChaCha20Rng) -> Result<String> {
    if let Some(len) = args.hex {
        let mut bytes = vec![0_u8; len];
        rng.fill_bytes(&mut bytes);
        return Ok(hex_encode(&bytes, false));
    }
    if let Some(len) = args.alnum {
        return Ok(random_from_charset(
            rng,
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
            len,
        ));
    }
    if let Some(len) = args.alpha {
        return Ok(random_from_charset(
            rng,
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
            len,
        ));
    }
    if let Some(len) = args.digits {
        return Ok(random_from_charset(rng, "0123456789", len));
    }
    if let Some(len) = args.base32 {
        let mut bytes = vec![0_u8; len];
        rng.fill_bytes(&mut bytes);
        return Ok(BASE32.encode(&bytes));
    }
    if let Some(len) = args.base64 {
        let mut bytes = vec![0_u8; len];
        rng.fill_bytes(&mut bytes);
        return Ok(B64.encode(&bytes));
    }
    if let Some(len) = args.ascii {
        let chars: String = (0x21_u8..=0x7e).map(char::from).collect();
        return Ok(random_from_charset(rng, &chars, len));
    }
    if let Some(charset) = &args.charset {
        let len = args
            .len
            .ok_or_else(|| PrismError::usage("--charset requires --len"))?;
        return Ok(random_from_charset(rng, charset, len));
    }
    if args.uuid {
        let mut bytes = [0_u8; 16];
        rng.fill_bytes(&mut bytes);
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        return Ok(Uuid::from_bytes(bytes).to_string());
    }
    if args.uuid7 {
        return Ok(uuid7_from_rng(rng, timestamp_ms(args, global)).to_string());
    }
    if args.ulid {
        return Ok(ulid_from_rng(rng, timestamp_ms(args, global)).to_string());
    }
    if let Some(count) = args.words {
        let words = load_wordlist(args.wordlist.as_deref())?;
        let mut selected = Vec::with_capacity(count);
        for _ in 0..count {
            let word = words
                .choose(rng)
                .ok_or_else(|| PrismError::runtime("wordlist is empty"))?;
            selected.push(word.clone());
        }
        return Ok(selected.join(&args.sep));
    }
    Err(PrismError::usage("choose a rand mode"))
}

fn random_from_charset(rng: &mut ChaCha20Rng, charset: &str, len: usize) -> String {
    let chars: Vec<char> = charset.chars().collect();
    let mut out = String::new();
    for _ in 0..len {
        if let Some(ch) = chars.choose(rng) {
            out.push(*ch);
        }
    }
    out
}

fn timestamp_ms(args: &RandArgs, global: &Global) -> u64 {
    if let Some(now) = args.now {
        return now;
    }
    if let Some(seed) = &global.seed {
        let digest = blake3::hash(seed.as_bytes());
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&digest.as_bytes()[..8]);
        return u64::from_le_bytes(bytes) % 4_102_444_800_000;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn uuid7_from_rng(rng: &mut ChaCha20Rng, timestamp_ms: u64) -> Uuid {
    let mut bytes = [0_u8; 16];
    rng.fill_bytes(&mut bytes[6..]);
    let ts = timestamp_ms & 0x0000_ffff_ffff_ffff;
    bytes[0] = (ts >> 40) as u8;
    bytes[1] = (ts >> 32) as u8;
    bytes[2] = (ts >> 24) as u8;
    bytes[3] = (ts >> 16) as u8;
    bytes[4] = (ts >> 8) as u8;
    bytes[5] = ts as u8;
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn ulid_from_rng(rng: &mut ChaCha20Rng, timestamp_ms: u64) -> ulid::Ulid {
    let mut random = [0_u8; 10];
    rng.fill_bytes(&mut random);
    let mut bytes = [0_u8; 16];
    let ts = timestamp_ms & 0x0000_ffff_ffff_ffff;
    bytes[0] = (ts >> 40) as u8;
    bytes[1] = (ts >> 32) as u8;
    bytes[2] = (ts >> 24) as u8;
    bytes[3] = (ts >> 16) as u8;
    bytes[4] = (ts >> 8) as u8;
    bytes[5] = ts as u8;
    bytes[6..].copy_from_slice(&random);
    ulid::Ulid::from_bytes(bytes)
}

fn load_wordlist(path: Option<&Path>) -> Result<Vec<String>> {
    if let Some(path) = path {
        return Ok(fs::read_to_string(path)?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
            .collect());
    }
    Ok([
        "amber", "beryl", "cinder", "dawn", "ember", "facet", "garnet", "halo", "ion", "jasper",
        "kelp", "lumen", "mica", "nova", "onyx", "pearl", "quartz", "rune", "sable", "topaz",
        "umber", "vapor", "wisp", "xylem", "yarrow", "zircon",
    ]
    .iter()
    .map(|word| (*word).to_string())
    .collect())
}

fn command_seq(args: &SeqArgs) -> Result<Vec<String>> {
    let (start, end) = args
        .range
        .split_once("..")
        .ok_or_else(|| PrismError::usage("sequence range must contain .."))?;
    let values = if let (Ok(a), Ok(b)) = (start.parse::<i64>(), end.parse::<i64>()) {
        let step = if a <= b { 1 } else { -1 };
        let mut values = Vec::new();
        let mut current = a;
        loop {
            values.push(format_seq_number(current, args));
            if current == b {
                break;
            }
            current += step;
        }
        values
    } else {
        alpha_seq(start, end)?
    };
    if let Some(sep) = &args.sep {
        Ok(vec![values.join(sep)])
    } else {
        Ok(values)
    }
}

fn format_seq_number(value: i64, args: &SeqArgs) -> String {
    if args.hex {
        return format!("{value:02x}");
    }
    if let Some(fmt) = &args.fmt {
        return apply_percent_d(fmt, value);
    }
    if let Some(width) = args.pad {
        return format!("{value:0width$}");
    }
    value.to_string()
}

fn apply_percent_d(fmt: &str, value: i64) -> String {
    if let Some(pos) = fmt.find('%') {
        if let Some(end) = fmt[pos + 1..].find('d') {
            let spec = &fmt[pos + 1..pos + 1 + end];
            let width = spec.trim_start_matches('0').parse::<usize>().ok();
            let formatted = if spec.starts_with('0') {
                width.map_or_else(|| value.to_string(), |w| format!("{value:0w$}"))
            } else {
                width.map_or_else(|| value.to_string(), |w| format!("{value:w$}"))
            };
            let mut out = String::new();
            out.push_str(&fmt[..pos]);
            out.push_str(&formatted);
            out.push_str(&fmt[pos + end + 2..]);
            return out;
        }
    }
    fmt.replace("%d", &value.to_string())
}

fn alpha_seq(start: &str, end: &str) -> Result<Vec<String>> {
    let a = alpha_to_num(start)?;
    let b = alpha_to_num(end)?;
    let width = start.len().max(end.len());
    let step = if a <= b { 1 } else { -1 };
    let mut out = Vec::new();
    let mut current = a;
    loop {
        out.push(num_to_alpha(current, width));
        if current == b {
            break;
        }
        current += step;
    }
    Ok(out)
}

fn alpha_to_num(value: &str) -> Result<i64> {
    let mut out = 0_i64;
    for ch in value.chars() {
        if !ch.is_ascii_lowercase() {
            return Err(PrismError::usage(
                "alphabetic sequences use lowercase ASCII letters",
            ));
        }
        out = out * 26 + i64::from(ch as u8 - b'a');
    }
    Ok(out)
}

fn num_to_alpha(mut value: i64, width: usize) -> String {
    let mut chars = vec!['a'; width];
    for idx in (0..width).rev() {
        chars[idx] = char::from(b'a' + (value.rem_euclid(26) as u8));
        value = value.div_euclid(26);
    }
    chars.into_iter().collect()
}

fn command_repeat(args: &RepeatArgs) -> String {
    let sep = args.sep.as_deref().unwrap_or("");
    std::iter::repeat(args.value.as_str())
        .take(args.count)
        .collect::<Vec<_>>()
        .join(sep)
}

fn command_pad(args: &PadArgs, global: &Global, input: Option<Data>) -> Result<Data> {
    if let Some(value) = &args.value {
        return Ok(Data::Records(vec![pad_record(value, args)]));
    }
    map_records(global, input, None, |record| Ok(pad_record(record, args)))
}

fn pad_record(record: &str, args: &PadArgs) -> String {
    let width = args
        .left
        .or(args.right)
        .or(args.center)
        .unwrap_or(record.width());
    let current = measure_width(record, args.width_mode);
    if current >= width {
        return record.to_string();
    }
    let needed = width - current;
    let fill = if args.fill.is_empty() {
        " "
    } else {
        &args.fill
    };
    if args.left.is_some() {
        return format!("{}{}", repeat_fill(fill, needed), record);
    }
    if args.center.is_some() {
        let left = needed / 2;
        let right = needed - left;
        return format!(
            "{}{}{}",
            repeat_fill(fill, left),
            record,
            repeat_fill(fill, right)
        );
    }
    format!("{}{}", record, repeat_fill(fill, needed))
}

fn measure_width(value: &str, mode: WidthMode) -> usize {
    match mode {
        WidthMode::Chars => value.chars().count(),
        WidthMode::Bytes => value.len(),
        WidthMode::Display => value.width(),
    }
}

fn repeat_fill(fill: &str, width: usize) -> String {
    fill.repeat(width)
}

fn words(value: &str) -> Vec<String> {
    let mut normalized = String::new();
    let mut prev_lower = false;
    for ch in value.chars() {
        if ch.is_alphanumeric() {
            if ch.is_uppercase() && prev_lower {
                normalized.push(' ');
            }
            normalized.push(ch);
            prev_lower = ch.is_lowercase() || ch.is_ascii_digit();
        } else {
            normalized.push(' ');
            prev_lower = false;
        }
    }
    normalized
        .split_whitespace()
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .collect()
}

fn convert_case(value: &str, mode: CaseMode) -> String {
    if value.is_ascii() {
        match mode {
            CaseMode::Upper => return value.to_ascii_uppercase(),
            CaseMode::Lower => return value.to_ascii_lowercase(),
            CaseMode::Snake => return ascii_words_join(value, "_", false),
            CaseMode::Kebab => return ascii_words_join(value, "-", false),
            CaseMode::Scream | CaseMode::Const => return ascii_words_join(value, "_", true),
            CaseMode::Dot => return ascii_words_join(value, ".", false),
            CaseMode::Path => return ascii_words_join(value, "/", false),
            _ => {}
        }
    }
    match mode {
        CaseMode::Upper => value.to_uppercase(),
        CaseMode::Lower => value.to_lowercase(),
        CaseMode::Swap => value
            .chars()
            .flat_map(|ch| {
                if ch.is_uppercase() {
                    ch.to_lowercase().collect::<Vec<_>>()
                } else {
                    ch.to_uppercase().collect::<Vec<_>>()
                }
            })
            .collect(),
        CaseMode::Snake => words(value).join("_"),
        CaseMode::Kebab => words(value).join("-"),
        CaseMode::Scream | CaseMode::Const => words(value).join("_").to_uppercase(),
        CaseMode::Dot => words(value).join("."),
        CaseMode::Path => words(value).join("/"),
        CaseMode::Title => words(value)
            .iter()
            .map(|word| capitalize(word))
            .collect::<Vec<_>>()
            .join(" "),
        CaseMode::Camel => {
            let parts = words(value);
            parts
                .iter()
                .enumerate()
                .map(|(idx, word)| {
                    if idx == 0 {
                        word.clone()
                    } else {
                        capitalize(word)
                    }
                })
                .collect()
        }
        CaseMode::Pascal => words(value).iter().map(|word| capitalize(word)).collect(),
    }
}

fn ascii_words_join(value: &str, sep: &str, upper: bool) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_sep = false;
    let mut prev_lower_or_digit = false;
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() {
            if (pending_sep || (byte.is_ascii_uppercase() && prev_lower_or_digit))
                && !out.is_empty()
            {
                out.push_str(sep);
            }
            let original = byte;
            let rendered = if upper {
                byte.to_ascii_uppercase()
            } else {
                byte.to_ascii_lowercase()
            };
            out.push(char::from(rendered));
            pending_sep = false;
            prev_lower_or_digit = original.is_ascii_lowercase() || original.is_ascii_digit();
        } else {
            if !out.is_empty() {
                pending_sep = true;
            }
            prev_lower_or_digit = false;
        }
    }
    out
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn slugify(value: &str, sep: &str, max: Option<usize>, keep_unicode: bool) -> String {
    if !keep_unicode && value.is_ascii() && max.is_none() {
        return slugify_ascii(value, sep);
    }
    let mut out = String::new();
    let mut pending_sep = false;
    for ch in value.nfkd().filter(|ch| !is_combining_mark(*ch)) {
        let lower = ch.to_lowercase().collect::<String>();
        for lower_ch in lower.chars() {
            let keep = lower_ch.is_ascii_lowercase()
                || lower_ch.is_ascii_digit()
                || (keep_unicode && lower_ch.is_alphabetic());
            if keep {
                if pending_sep && !out.is_empty() {
                    out.push_str(sep);
                }
                out.push(lower_ch);
                pending_sep = false;
            } else if !out.is_empty() {
                pending_sep = true;
            }
            if max.is_some_and(|limit| out.chars().count() >= limit) {
                return out.trim_matches(|c| sep.contains(c)).to_string();
            }
        }
    }
    out.trim_matches(|c| sep.contains(c)).to_string()
}

fn slugify_ascii(value: &str, sep: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_sep = false;
    for byte in value.bytes() {
        let next = match byte {
            b'a'..=b'z' | b'0'..=b'9' => Some(char::from(byte)),
            b'A'..=b'Z' => Some(char::from(byte.to_ascii_lowercase())),
            _ => None,
        };
        if let Some(ch) = next {
            if pending_sep && !out.is_empty() {
                out.push_str(sep);
            }
            out.push(ch);
            pending_sep = false;
        } else if !out.is_empty() {
            pending_sep = true;
        }
    }
    out
}

fn trim_record(record: &str, args: &TrimArgs) -> String {
    let trim_left = args.left || !args.right;
    let trim_right = args.right || !args.left;
    let predicate = |ch: char| -> bool {
        if let Some(chars) = &args.chars {
            chars.contains(ch)
        } else if args.ascii {
            ch.is_ascii_whitespace()
        } else {
            ch.is_whitespace()
        }
    };
    match (trim_left, trim_right) {
        (true, true) => record.trim_matches(predicate).to_string(),
        (true, false) => record.trim_start_matches(predicate).to_string(),
        (false, true) => record.trim_end_matches(predicate).to_string(),
        (false, false) => record.to_string(),
    }
}

fn compile_squeeze_regex(args: &SqueezeArgs) -> Result<Option<Regex>> {
    args.char
        .as_ref()
        .map(|ch| {
            let pattern = regex::escape(ch);
            Regex::new(&format!("(?:{pattern}){{2,}}"))
                .map_err(|err| PrismError::usage(format!("invalid squeeze pattern: {err}")))
        })
        .transpose()
}

fn squeeze_record(record: &str, args: &SqueezeArgs, repeated_char: Option<&Regex>) -> String {
    if let (Some(ch), Some(regex)) = (&args.char, repeated_char) {
        return regex.replace_all(record, ch.as_str()).to_string();
    }
    let mut out = String::new();
    let mut in_space = false;
    for ch in record.chars() {
        let is_space = if args.ascii {
            ch.is_ascii_whitespace()
        } else {
            ch.is_whitespace()
        };
        if is_space {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(ch);
            in_space = false;
        }
    }
    out
}

fn wrap_text(value: &str, width: usize, hanging: usize, mode: WidthMode) -> String {
    let mut lines = Vec::new();
    let mut current = String::new();
    let hanging_pad = " ".repeat(hanging);
    for word in value.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if measure_width(&candidate, mode) > width && !current.is_empty() {
            lines.push(current);
            current = format!("{hanging_pad}{word}");
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines.join("\n")
}

fn indent_text(value: &str, spaces: usize, tabs: usize) -> String {
    let prefix = format!("{}{}", "\t".repeat(tabs), " ".repeat(spaces));
    value
        .lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn dedent_text(value: &str) -> String {
    let min_indent = value
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.chars()
                .take_while(|ch| *ch == ' ' || *ch == '\t')
                .count()
        })
        .min()
        .unwrap_or(0);
    value
        .lines()
        .map(|line| line.chars().skip(min_indent).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn replace_regex_record(record: &str, args: &ReplaceArgs, regex: &Regex) -> String {
    if args.first {
        regex.replace(record, args.replacement.as_str()).to_string()
    } else {
        regex
            .replace_all(record, args.replacement.as_str())
            .to_string()
    }
}

fn replace_literal_record(record: &str, args: &ReplaceArgs) -> String {
    if args.first {
        record.replacen(&args.needle, &args.replacement, 1)
    } else {
        record.replace(&args.needle, &args.replacement)
    }
}

fn field_record(
    record: &str,
    args: &FieldArgs,
    selections: &[FieldSelection],
    osep: &str,
    delimiter_regex: Option<&Regex>,
) -> Result<String> {
    if let Some(value) =
        field_record_literal_positive(record, args, selections, osep, delimiter_regex)?
    {
        return Ok(value);
    }
    let fields = split_fields(record, args, delimiter_regex);
    let mut out: Vec<&str> = Vec::new();
    for selection in selections.iter().copied() {
        match selection {
            FieldSelection::Index(index) => {
                if index == 0 {
                    return Err(PrismError::usage(
                        "fields are 1-based; did you mean -1 for last?",
                    ));
                }
                match resolve_field_index(index, fields.len()) {
                    Some(idx) => out.push(fields[idx]),
                    None if args.strict_fields => {
                        return Err(PrismError::runtime(format!(
                            "field {index} is out of range"
                        )));
                    }
                    None => out.push(""),
                }
            }
            FieldSelection::Range(start, end) => {
                let start_idx = match start {
                    Some(value) => resolve_field_index(value, fields.len()),
                    None => Some(0),
                };
                let end_idx = match end {
                    Some(value) => resolve_field_index(value, fields.len()),
                    None => fields.len().checked_sub(1),
                };
                match (start_idx, end_idx) {
                    (Some(a), Some(b)) if a <= b => {
                        out.extend(fields[a..=b].iter().copied());
                    }
                    _ if args.strict_fields => {
                        return Err(PrismError::runtime("field range is out of range"))
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(out.join(osep))
}

#[derive(Debug, Clone, Copy)]
enum PositiveFieldSelection {
    Index(usize),
    Range { start: usize, end: Option<usize> },
}

fn field_record_literal_positive<'a>(
    record: &'a str,
    args: &FieldArgs,
    selections: &[FieldSelection],
    osep: &str,
    delimiter_regex: Option<&Regex>,
) -> Result<Option<String>> {
    if args.strict_fields || delimiter_regex.is_some() || args.regex {
        return Ok(None);
    }
    let Some(delimiter) = args.delimiter.as_deref() else {
        return Ok(None);
    };
    if delimiter.is_empty() {
        return Ok(None);
    }
    let mut positive = Vec::with_capacity(selections.len());
    for selection in selections.iter().copied() {
        let Some(selection) = positive_field_selection(selection)? else {
            return Ok(None);
        };
        positive.push(selection);
    }

    let mut selected: Vec<Vec<&'a str>> = (0..positive.len()).map(|_| Vec::new()).collect();
    let mut field_count = 0;
    for (idx, field) in record.split(delimiter).enumerate() {
        let field_no = idx + 1;
        field_count = field_no;
        for (selection_idx, selection) in positive.iter().copied().enumerate() {
            match selection {
                PositiveFieldSelection::Index(index) if index == field_no => {
                    selected[selection_idx].push(field);
                }
                PositiveFieldSelection::Range { start, end }
                    if field_no >= start && end.map_or(true, |value| field_no <= value) =>
                {
                    selected[selection_idx].push(field);
                }
                _ => {}
            }
        }
    }

    let mut out = String::new();
    let mut emitted = false;
    for (selection, values) in positive.iter().copied().zip(selected) {
        match selection {
            PositiveFieldSelection::Index(_) => {
                push_joined_field(
                    &mut out,
                    &mut emitted,
                    values.first().copied().unwrap_or(""),
                    osep,
                );
            }
            PositiveFieldSelection::Range { .. } => {
                if positive_range_resolves(selection, field_count) {
                    for value in values {
                        push_joined_field(&mut out, &mut emitted, value, osep);
                    }
                }
            }
        }
    }
    Ok(Some(out))
}

fn positive_range_resolves(selection: PositiveFieldSelection, field_count: usize) -> bool {
    match selection {
        PositiveFieldSelection::Range { start, end } => match end {
            Some(end) => start <= end && end <= field_count,
            None => start <= field_count && field_count > 0,
        },
        PositiveFieldSelection::Index(_) => true,
    }
}

fn positive_field_selection(selection: FieldSelection) -> Result<Option<PositiveFieldSelection>> {
    match selection {
        FieldSelection::Index(0) => Err(PrismError::usage(
            "fields are 1-based; did you mean -1 for last?",
        )),
        FieldSelection::Index(index) if index > 0 => Ok(Some(PositiveFieldSelection::Index(
            usize::try_from(index)
                .map_err(|err| PrismError::usage(format!("invalid field index: {err}")))?,
        ))),
        FieldSelection::Index(_) => Ok(None),
        FieldSelection::Range(start, end) => {
            let start = match start {
                Some(value) if value <= 0 => return Ok(None),
                Some(value) => usize::try_from(value)
                    .map_err(|err| PrismError::usage(format!("invalid field range: {err}")))?,
                None => 1,
            };
            let end = match end {
                Some(value) if value <= 0 => return Ok(None),
                Some(value) => Some(
                    usize::try_from(value)
                        .map_err(|err| PrismError::usage(format!("invalid field range: {err}")))?,
                ),
                None => None,
            };
            Ok(Some(PositiveFieldSelection::Range { start, end }))
        }
    }
}

fn push_joined_field(out: &mut String, emitted: &mut bool, value: &str, osep: &str) {
    if *emitted {
        out.push_str(osep);
    }
    out.push_str(value);
    *emitted = true;
}

fn compile_field_delimiter_regex(args: &FieldArgs) -> Result<Option<Regex>> {
    if !args.regex {
        return Ok(None);
    }
    let Some(delimiter) = &args.delimiter else {
        return Ok(None);
    };
    Regex::new(delimiter)
        .map(Some)
        .map_err(|err| PrismError::usage(format!("invalid delimiter regex: {err}")))
}

fn split_fields<'a>(
    record: &'a str,
    args: &FieldArgs,
    delimiter_regex: Option<&Regex>,
) -> Vec<&'a str> {
    if let Some(delimiter) = &args.delimiter {
        if let Some(regex) = delimiter_regex {
            return regex.split(record).collect();
        }
        return record.split(delimiter).collect();
    }
    record.split_whitespace().collect()
}

#[derive(Debug, Clone, Copy)]
enum FieldSelection {
    Index(isize),
    Range(Option<isize>, Option<isize>),
}

fn parse_field_spec(spec: &str) -> Result<Vec<FieldSelection>> {
    spec.split(',')
        .map(|part| {
            if let Some((start, end)) = part.split_once("..") {
                let start = if start.is_empty() {
                    None
                } else {
                    Some(parse_field_index(start)?)
                };
                let end = if end.is_empty() {
                    None
                } else {
                    Some(parse_field_index(end)?)
                };
                Ok(FieldSelection::Range(start, end))
            } else {
                Ok(FieldSelection::Index(parse_field_index(part)?))
            }
        })
        .collect()
}

fn parse_field_index(value: &str) -> Result<isize> {
    value
        .parse::<isize>()
        .map_err(|err| PrismError::usage(format!("invalid field index {value}: {err}")))
}

fn resolve_field_index(index: isize, len: usize) -> Option<usize> {
    if index > 0 {
        usize::try_from(index - 1).ok().filter(|idx| *idx < len)
    } else if index < 0 {
        let len = isize::try_from(len).ok()?;
        let len_usize = usize::try_from(len).ok()?;
        usize::try_from(len + index)
            .ok()
            .filter(|idx| *idx < len_usize)
    } else {
        None
    }
}

fn slice_record(record: &str, args: &SliceArgs, selection: SliceSelection) -> Result<String> {
    if args.bytes && args.graphemes {
        return Err(PrismError::usage(
            "--bytes and --graphemes are mutually exclusive",
        ));
    }
    if args.bytes {
        return slice_bytes(record, selection);
    }
    if !args.graphemes {
        return slice_chars(record, selection);
    }
    let units: Vec<String> = { record.graphemes(true).map(ToString::to_string).collect() };
    let selected = slice_units(&units, selection);
    Ok(selected.concat())
}

#[derive(Debug, Clone, Copy)]
enum SliceSelection {
    Index(isize),
    Range(Option<isize>, Option<isize>),
}

fn parse_slice_spec(spec: &str) -> Result<SliceSelection> {
    if let Some((start, end)) = spec.split_once("..") {
        let start = if start.is_empty() {
            None
        } else {
            Some(
                start
                    .parse()
                    .map_err(|err| PrismError::usage(format!("invalid slice start: {err}")))?,
            )
        };
        let end = if end.is_empty() {
            None
        } else {
            Some(
                end.parse()
                    .map_err(|err| PrismError::usage(format!("invalid slice end: {err}")))?,
            )
        };
        Ok(SliceSelection::Range(start, end))
    } else {
        Ok(SliceSelection::Index(spec.parse().map_err(|err| {
            PrismError::usage(format!("invalid slice index: {err}"))
        })?))
    }
}

fn slice_bytes(record: &str, selection: SliceSelection) -> Result<String> {
    let len = record.len();
    let (start, end) = match selection {
        SliceSelection::Range(start, end) => {
            let start = start
                .map_or(0, |idx| resolve_slice_index(idx, len))
                .min(len);
            let end = end
                .map_or(len, |idx| resolve_slice_index(idx, len))
                .min(len);
            (start, end)
        }
        SliceSelection::Index(idx) => {
            let start = resolve_slice_index(idx, len).min(len);
            (start, (start + 1).min(len))
        }
    };
    if start >= end {
        return Ok(String::new());
    }
    if !record.is_char_boundary(start) || !record.is_char_boundary(end) {
        return Err(PrismError::runtime(
            "byte slice does not fall on valid UTF-8 boundaries",
        ));
    }
    Ok(record[start..end].to_string())
}

fn slice_chars(record: &str, selection: SliceSelection) -> Result<String> {
    let len = record.chars().count();
    let (start, end) = match selection {
        SliceSelection::Range(start, end) => {
            let start = start
                .map_or(0, |idx| resolve_slice_index(idx, len))
                .min(len);
            let end = end
                .map_or(len, |idx| resolve_slice_index(idx, len))
                .min(len);
            (start, end)
        }
        SliceSelection::Index(idx) => {
            let start = resolve_slice_index(idx, len).min(len);
            (start, (start + 1).min(len))
        }
    };
    if start >= end {
        return Ok(String::new());
    }
    let start_byte = char_byte_index(record, start);
    let end_byte = char_byte_index(record, end);
    Ok(record[start_byte..end_byte].to_string())
}

fn char_byte_index(record: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }
    record
        .char_indices()
        .nth(char_index)
        .map(|(idx, _)| idx)
        .unwrap_or(record.len())
}

fn slice_units(units: &[String], selection: SliceSelection) -> Vec<String> {
    match selection {
        SliceSelection::Range(start, end) => {
            let start = start
                .map_or(0, |idx| resolve_slice_index(idx, units.len()))
                .min(units.len());
            let end = end
                .map_or(units.len(), |idx| resolve_slice_index(idx, units.len()))
                .min(units.len());
            if start >= end {
                Vec::new()
            } else {
                units[start..end].to_vec()
            }
        }
        SliceSelection::Index(idx) => {
            let idx = resolve_slice_index(idx, units.len());
            units.get(idx).cloned().into_iter().collect()
        }
    }
}

fn resolve_slice_index(index: isize, len: usize) -> usize {
    if index >= 0 {
        index as usize
    } else {
        let len = len as isize;
        (len + index).max(0) as usize
    }
}

fn lines_records(
    mut records: Vec<String>,
    args: &LinesArgs,
    global: &Global,
) -> Result<Vec<String>> {
    if args.number {
        records = records
            .into_iter()
            .enumerate()
            .map(|(idx, record)| format!("{}\t{}", idx + 1, record))
            .collect();
    }
    if args.uniq {
        let mut out = Vec::new();
        let mut prev: Option<String> = None;
        for record in records {
            if prev.as_deref() != Some(record.as_str()) {
                prev = Some(record.clone());
                out.push(record);
            }
        }
        records = out;
    }
    if args.uniq_global {
        let mut seen = HashSet::new();
        records.retain(|record| seen.insert(record.clone()));
    }
    if args.sort {
        if args.numeric {
            let mut keyed: Vec<(f64, String)> = records
                .into_iter()
                .map(|record| (record.trim().parse::<f64>().unwrap_or(0.0), record))
                .collect();
            keyed.sort_by(|(a_num, _), (b_num, _)| {
                a_num
                    .partial_cmp(b_num)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            records = keyed.into_iter().map(|(_, record)| record).collect();
        } else {
            records.sort();
        }
    }
    if args.shuffle {
        let mut rng = rng_from_global(global, "lines --shuffle");
        records.shuffle(&mut rng);
    }
    if args.reverse {
        records.reverse();
    }
    Ok(records)
}

fn command_enc(args: &EncArgs, global: &Global, input: Option<Data>) -> Result<Data> {
    if args.per_line {
        return map_records(global, input, args.value.as_deref(), |record| {
            enc_value(record.as_bytes(), args).and_then(|bytes| {
                String::from_utf8(bytes).map_err(|err| {
                    PrismError::runtime(format!("encoded value is not UTF-8: {err}"))
                })
            })
        });
    }
    let bytes = input_bytes(global, input, args.value.as_deref())?;
    let encoded = enc_value(&bytes, args)?;
    if args.decode {
        return Ok(Data::Bytes(encoded));
    }
    String::from_utf8(encoded)
        .map(|value| Data::Records(vec![value]))
        .map_err(|err| PrismError::runtime(format!("encoded value is not UTF-8: {err}")))
}

fn enc_value(bytes: &[u8], args: &EncArgs) -> Result<Vec<u8>> {
    let codec = args.codec.as_str();
    if args.decode {
        decode_value(bytes, codec, args)
    } else {
        encode_value(bytes, codec, args)
    }
}

fn encode_value(bytes: &[u8], codec: &str, args: &EncArgs) -> Result<Vec<u8>> {
    let text = || {
        String::from_utf8(bytes.to_vec())
            .map_err(|err| PrismError::runtime(format!("invalid UTF-8: {err}")))
    };
    let out = match codec {
        "base64" => {
            if args.no_pad {
                B64.encode(bytes).trim_end_matches('=').to_string()
            } else {
                B64.encode(bytes)
            }
        }
        "base64url" => {
            if args.no_pad {
                URL_SAFE_NO_PAD.encode(bytes)
            } else {
                B64_URL.encode(bytes)
            }
        }
        "base32" => BASE32.encode(bytes),
        "base32hex" => BASE32HEX.encode(bytes),
        "base16" | "hex" => hex_encode(bytes, args.upper),
        "url" => {
            let value = text()?;
            if args.component {
                utf8_percent_encode(&value, URL_COMPONENT_ENCODE).to_string()
            } else {
                utf8_percent_encode(&value, URL_ENCODE).to_string()
            }
        }
        "html" => escape_html(&text()?),
        "xml" => escape_xml(&text()?),
        "quoted-printable" => quoted_printable::encode_to_str(bytes),
        "ascii85" | "base85" => ascii85_encode(bytes),
        "rot13" => rot13(&text()?),
        "punycode" => idna::domain_to_ascii(&text()?)
            .map_err(|err| PrismError::runtime(format!("punycode encode failed: {err}")))?,
        "shell" => quote_shell(&text()?),
        "json" => serde_json::to_string(&text()?)
            .map_err(|err| PrismError::runtime(format!("json encode failed: {err}")))?,
        "csv-field" => csv_quote(&text()?),
        other => return Err(PrismError::usage(format!("unknown codec: {other}"))),
    };
    Ok(out.into_bytes())
}

fn decode_value(bytes: &[u8], codec: &str, args: &EncArgs) -> Result<Vec<u8>> {
    let value = String::from_utf8(bytes.to_vec())
        .map_err(|err| PrismError::runtime(format!("invalid UTF-8: {err}")))?;
    match codec {
        "base64" => B64
            .decode(pad_base64(value.trim()).as_bytes())
            .map_err(|err| PrismError::runtime(format!("base64 decode failed: {err}"))),
        "base64url" => B64_URL
            .decode(value.trim())
            .or_else(|_| URL_SAFE_NO_PAD.decode(value.trim()))
            .map_err(|err| PrismError::runtime(format!("base64url decode failed: {err}"))),
        "base32" => BASE32
            .decode(value.trim().as_bytes())
            .map_err(|err| PrismError::runtime(format!("base32 decode failed: {err}"))),
        "base32hex" => BASE32HEX
            .decode(value.trim().as_bytes())
            .map_err(|err| PrismError::runtime(format!("base32hex decode failed: {err}"))),
        "base16" | "hex" => hex_decode(value.trim()),
        "url" => percent_decode(value.as_bytes())
            .decode_utf8()
            .map(|decoded| decoded.into_owned().into_bytes())
            .map_err(|err| PrismError::runtime(format!("url decode failed: {err}"))),
        "html" | "xml" => Ok(unescape_entities(&value).into_bytes()),
        "quoted-printable" => {
            quoted_printable::decode(value.as_bytes(), quoted_printable::ParseMode::Robust).map_err(
                |err| PrismError::runtime(format!("quoted-printable decode failed: {err}")),
            )
        }
        "ascii85" | "base85" => ascii85_decode(value.trim()),
        "rot13" => Ok(rot13(&value).into_bytes()),
        "punycode" => Ok(idna::domain_to_unicode(&value).0.into_bytes()),
        "json" => serde_json::from_str::<String>(&value)
            .map(|decoded| decoded.into_bytes())
            .map_err(|err| PrismError::runtime(format!("json decode failed: {err}"))),
        "csv-field" => Ok(csv_unquote(&value).into_bytes()),
        "shell" => Err(PrismError::usage("shell codec is encode-only")),
        other => Err(PrismError::usage(format!("unknown codec: {other}"))),
    }
    .map(|mut out| {
        if args.no_pad && matches!(codec, "base64" | "base64url") {
            out.shrink_to_fit();
        }
        out
    })
}

fn pad_base64(value: &str) -> String {
    let mut padded = value.to_string();
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    padded
}

fn hex_encode(bytes: &[u8], upper: bool) -> String {
    if upper {
        HEXUPPER.encode(bytes)
    } else {
        HEXLOWER.encode(bytes)
    }
}

fn hex_decode(value: &str) -> Result<Vec<u8>> {
    let normalized = value.to_ascii_lowercase();
    HEXLOWER
        .decode(normalized.as_bytes())
        .map_err(|err| PrismError::runtime(format!("hex decode failed: {err}")))
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn unescape_entities(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn ascii85_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(4) {
        let mut block = [0_u8; 4];
        block[..chunk.len()].copy_from_slice(chunk);
        let mut value = u32::from_be_bytes(block);
        let mut encoded = [0_u8; 5];
        for idx in (0..5).rev() {
            encoded[idx] = (value % 85 + 33) as u8;
            value /= 85;
        }
        let take = if chunk.len() < 4 { chunk.len() + 1 } else { 5 };
        out.push_str(&String::from_utf8_lossy(&encoded[..take]));
    }
    out
}

fn ascii85_decode(value: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for chunk in value.as_bytes().chunks(5) {
        let mut digits = [b'u'; 5];
        digits[..chunk.len()].copy_from_slice(chunk);
        let mut acc = 0_u32;
        for digit in digits {
            if !(33..=117).contains(&digit) {
                return Err(PrismError::runtime("invalid ascii85 digit"));
            }
            acc = acc * 85 + u32::from(digit - 33);
        }
        let bytes = acc.to_be_bytes();
        let take = if chunk.len() < 5 {
            chunk.len().saturating_sub(1)
        } else {
            4
        };
        out.extend_from_slice(&bytes[..take]);
    }
    Ok(out)
}

fn rot13(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' => char::from(b'a' + ((ch as u8 - b'a' + 13) % 26)),
            'A'..='Z' => char::from(b'A' + ((ch as u8 - b'A' + 13) % 26)),
            _ => ch,
        })
        .collect()
}

fn quote_shell(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn csv_quote(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn csv_unquote(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        trimmed[1..trimmed.len() - 1].replace("\"\"", "\"")
    } else {
        trimmed.to_string()
    }
}

fn command_hash(args: &HashArgs, global: &Global, input: Option<Data>) -> Result<Data> {
    if args.raw {
        let bytes = input_bytes(global, input, args.value.as_deref())?;
        return Ok(Data::Bytes(hash_bytes(
            &bytes,
            &args.algorithm,
            args.key.as_deref(),
        )?));
    }
    if args.per_line {
        return map_records(global, input, args.value.as_deref(), |record| {
            format_digest(
                hash_bytes(record.as_bytes(), &args.algorithm, args.key.as_deref())?,
                args,
            )
        });
    }
    let bytes = input_bytes(global, input, args.value.as_deref())?;
    let digest = hash_bytes(&bytes, &args.algorithm, args.key.as_deref())?;
    Ok(Data::Records(vec![format_digest(digest, args)?]))
}

fn hash_bytes(bytes: &[u8], algorithm: &str, key: Option<&str>) -> Result<Vec<u8>> {
    if let Some(rest) = algorithm.strip_prefix("hmac-") {
        let key = key.ok_or_else(|| PrismError::usage("hmac requires --key"))?;
        return hmac_digest(rest, key.as_bytes(), bytes);
    }
    match algorithm {
        "md5" => Ok(Md5Digest::digest(bytes).to_vec()),
        "sha1" => Ok(Sha1::digest(bytes).to_vec()),
        "sha224" => Ok(Sha224::digest(bytes).to_vec()),
        "sha256" => Ok(Sha256::digest(bytes).to_vec()),
        "sha384" => Ok(Sha384::digest(bytes).to_vec()),
        "sha512" => Ok(Sha512::digest(bytes).to_vec()),
        "sha3-256" => Ok(Sha3_256::digest(bytes).to_vec()),
        "sha3-512" => Ok(Sha3_512::digest(bytes).to_vec()),
        "blake2b" => Ok(Blake2b512::digest(bytes).to_vec()),
        "blake2s" => Ok(Blake2s256::digest(bytes).to_vec()),
        "blake3" => Ok(blake3::hash(bytes).as_bytes().to_vec()),
        "crc32" => {
            let mut hasher = Crc32::new();
            hasher.update(bytes);
            Ok(hasher.finalize().to_be_bytes().to_vec())
        }
        "xxh64" => Ok(xxh64(bytes, 0).to_be_bytes().to_vec()),
        "xxh3" => Ok(xxh3_64(bytes).to_be_bytes().to_vec()),
        "fnv1a" => Ok(fnv1a(bytes).to_be_bytes().to_vec()),
        other => Err(PrismError::usage(format!(
            "unknown hash algorithm: {other}"
        ))),
    }
}

type Md5Digest = md5::Md5;

fn hmac_digest(algorithm: &str, key: &[u8], bytes: &[u8]) -> Result<Vec<u8>> {
    macro_rules! hmac_for {
        ($ty:ty) => {{
            let mut mac = <Hmac<$ty> as Mac>::new_from_slice(key)
                .map_err(|err| PrismError::runtime(format!("invalid hmac key: {err}")))?;
            mac.update(bytes);
            Ok(mac.finalize().into_bytes().to_vec())
        }};
    }
    match algorithm {
        "sha1" => hmac_for!(Sha1),
        "sha224" => hmac_for!(Sha224),
        "sha256" => hmac_for!(Sha256),
        "sha384" => hmac_for!(Sha384),
        "sha512" => hmac_for!(Sha512),
        "sha3-256" => hmac_for!(Sha3_256),
        "sha3-512" => hmac_for!(Sha3_512),
        other => Err(PrismError::usage(format!(
            "unsupported hmac algorithm: {other}"
        ))),
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn format_digest(mut digest: Vec<u8>, args: &HashArgs) -> Result<String> {
    let mut out = if args.base64 {
        B64.encode(&digest)
    } else {
        hex_encode(&digest, args.upper)
    };
    if let Some(short) = args.short {
        out = out.chars().take(short).collect();
    }
    digest.clear();
    Ok(out)
}

fn render_template(input: &str, args: &TplArgs, global: &Global) -> Result<String> {
    let vars = template_vars(args)?;
    if args.recursive {
        let mut current = input.to_string();
        let mut seen = HashSet::new();
        for depth in 0..args.max_depth {
            if !seen.insert(current.clone()) {
                return Err(PrismError::recursion("template recursion cycle detected"));
            }
            let next = expand_template_once(&current, args, global, &vars, &[])?;
            if next == current {
                if current.contains("${") {
                    return Err(PrismError::recursion("template recursion cycle detected"));
                }
                return Ok(next);
            }
            current = next;
            if depth + 1 == args.max_depth {
                return Err(PrismError::recursion("template recursion depth exceeded"));
            }
        }
        Err(PrismError::recursion("template recursion depth exceeded"))
    } else {
        expand_template_once(input, args, global, &vars, &[])
    }
}

fn template_vars(args: &TplArgs) -> Result<HashMap<String, String>> {
    let mut vars = HashMap::new();
    if args.env_file_override {
        vars.extend(env::vars());
        vars.extend(read_dotenv(args.env_file.as_deref())?);
    } else {
        vars.extend(read_dotenv(args.env_file.as_deref())?);
        vars.extend(env::vars());
    }
    for item in &args.set {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| PrismError::usage("--set requires KEY=VALUE"))?;
        vars.insert(key.to_string(), value.to_string());
    }
    Ok(vars)
}

fn read_dotenv(path: Option<&Path>) -> Result<HashMap<String, String>> {
    let mut vars = HashMap::new();
    let Some(path) = path else {
        return Ok(vars);
    };
    for line in fs::read_to_string(path)?.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            vars.insert(key.trim().to_string(), value.trim_matches('"').to_string());
        }
    }
    Ok(vars)
}

fn expand_template_once(
    input: &str,
    args: &TplArgs,
    global: &Global,
    vars: &HashMap<String, String>,
    stack: &[String],
) -> Result<String> {
    let mut out = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut idx = 0;
    while idx < chars.len() {
        if chars[idx] == '$' && chars.get(idx + 1) == Some(&'{') {
            let start = idx + 2;
            let mut depth = 1;
            let mut end = start;
            while end < chars.len() {
                if chars[end] == '{' {
                    depth += 1;
                } else if chars[end] == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                end += 1;
            }
            if end >= chars.len() {
                return Err(PrismError::usage("unterminated template placeholder"));
            }
            let expr: String = chars[start..end].iter().collect();
            out.push_str(&expand_placeholder(&expr, args, global, vars, stack)?);
            idx = end + 1;
        } else {
            out.push(chars[idx]);
            idx += 1;
        }
    }
    Ok(out)
}

fn expand_placeholder(
    expr: &str,
    args: &TplArgs,
    global: &Global,
    vars: &HashMap<String, String>,
    stack: &[String],
) -> Result<String> {
    if let Some(helper) = expr.strip_prefix('@') {
        if args.no_gen {
            return Err(PrismError::usage("@ helpers are disabled by --no-gen"));
        }
        let helper = if helper.contains("${") {
            expand_template_once(helper, args, global, vars, stack)?
        } else {
            helper.to_string()
        };
        return expand_helper(&helper, global);
    }
    let (key, op, operand) = split_template_expr(expr);
    if stack.iter().any(|existing| existing == key) {
        return Err(PrismError::recursion(format!("template cycle at {key}")));
    }
    let value = vars.get(key).cloned().unwrap_or_default();
    match op {
        Some(":-") => {
            if value.is_empty() {
                Ok(operand.unwrap_or_default().to_string())
            } else {
                Ok(value)
            }
        }
        Some(":?") => {
            if value.is_empty() {
                Err(PrismError::template(
                    operand.unwrap_or("required variable missing").to_string(),
                ))
            } else {
                Ok(value)
            }
        }
        Some(":+") => {
            if value.is_empty() {
                Ok(String::new())
            } else {
                Ok(operand.unwrap_or_default().to_string())
            }
        }
        Some(other) => Err(PrismError::usage(format!(
            "unsupported template operator: {other}"
        ))),
        None if value.is_empty() && args.strict => {
            Err(PrismError::template(format!("{key} is required")))
        }
        None => Ok(value),
    }
}

fn split_template_expr(expr: &str) -> (&str, Option<&str>, Option<&str>) {
    for op in [":-", ":?", ":+"] {
        if let Some((key, value)) = expr.split_once(op) {
            return (key, Some(op), Some(value));
        }
    }
    (expr, None, None)
}

fn expand_helper(helper: &str, global: &Global) -> Result<String> {
    let parts: Vec<&str> = helper.split(':').collect();
    match parts.as_slice() {
        ["uuid"] => {
            let args = RandArgs {
                hex: None,
                alnum: None,
                alpha: None,
                digits: None,
                base32: None,
                base64: None,
                ascii: None,
                charset: None,
                len: None,
                uuid: true,
                uuid7: false,
                ulid: false,
                words: None,
                sep: " ".to_string(),
                wordlist: None,
                bytes: None,
                now: None,
            };
            let mut rng = rng_from_global(global, "tpl @uuid");
            rand_text(&args, global, &mut rng)
        }
        ["uuid7"] => {
            let args = RandArgs {
                hex: None,
                alnum: None,
                alpha: None,
                digits: None,
                base32: None,
                base64: None,
                ascii: None,
                charset: None,
                len: None,
                uuid: false,
                uuid7: true,
                ulid: false,
                words: None,
                sep: " ".to_string(),
                wordlist: None,
                bytes: None,
                now: None,
            };
            let mut rng = rng_from_global(global, "tpl @uuid7");
            rand_text(&args, global, &mut rng)
        }
        ["ulid"] => {
            let args = RandArgs {
                hex: None,
                alnum: None,
                alpha: None,
                digits: None,
                base32: None,
                base64: None,
                ascii: None,
                charset: None,
                len: None,
                uuid: false,
                uuid7: false,
                ulid: true,
                words: None,
                sep: " ".to_string(),
                wordlist: None,
                bytes: None,
                now: None,
            };
            let mut rng = rng_from_global(global, "tpl @ulid");
            rand_text(&args, global, &mut rng)
        }
        ["now"] => Ok(Utc::now().to_rfc3339()),
        ["now", fmt] => Ok(Utc::now().format(fmt).to_string()),
        ["now", offset, fmt] => {
            let dt = Utc::now().fixed_offset();
            Ok(apply_offset(dt, offset)?.format(fmt).to_string())
        }
        ["rand", "hex", len] => helper_rand_hex(global, len),
        ["rand", "alnum", len] => helper_rand_alnum(global, len),
        ["rand", "words", len] => helper_rand_words(global, len),
        ["slug", text] => Ok(slugify(text, "-", None, false)),
        ["case", "snake", text] => Ok(convert_case(text, CaseMode::Snake)),
        ["enc", "base64", text] => Ok(B64.encode(text.as_bytes())),
        ["hash", "sha256", text] => Ok(hex_encode(&Sha256::digest(text.as_bytes()), false)),
        _ => Err(PrismError::usage(format!(
            "unsupported template helper: @{helper}"
        ))),
    }
}

fn helper_rand_hex(global: &Global, len: &str) -> Result<String> {
    let len = len
        .parse::<usize>()
        .map_err(|err| PrismError::usage(format!("invalid rand length: {err}")))?;
    let mut rng = rng_from_global(global, "tpl @rand:hex");
    let mut bytes = vec![0_u8; len];
    rng.fill_bytes(&mut bytes);
    Ok(hex_encode(&bytes, false))
}

fn helper_rand_alnum(global: &Global, len: &str) -> Result<String> {
    let len = len
        .parse::<usize>()
        .map_err(|err| PrismError::usage(format!("invalid rand length: {err}")))?;
    let mut rng = rng_from_global(global, "tpl @rand:alnum");
    Ok(random_from_charset(
        &mut rng,
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
        len,
    ))
}

fn helper_rand_words(global: &Global, len: &str) -> Result<String> {
    let len = len
        .parse::<usize>()
        .map_err(|err| PrismError::usage(format!("invalid rand length: {err}")))?;
    let mut rng = rng_from_global(global, "tpl @rand:words");
    let words = load_wordlist(None)?;
    let mut selected = Vec::with_capacity(len);
    for _ in 0..len {
        let word = words
            .choose(&mut rng)
            .ok_or_else(|| PrismError::runtime("wordlist is empty"))?;
        selected.push(word.clone());
    }
    Ok(selected.join(" "))
}

fn quote_value(value: &str, kind: QuoteKind) -> Result<String> {
    match kind {
        QuoteKind::Shell => Ok(quote_shell(value)),
        QuoteKind::Json => serde_json::to_string(value)
            .map_err(|err| PrismError::runtime(format!("json quote failed: {err}"))),
        QuoteKind::C => serde_json::to_string(value)
            .map_err(|err| PrismError::runtime(format!("c quote failed: {err}"))),
        QuoteKind::Regex => Ok(regex::escape(value)),
        QuoteKind::Sql => Ok(format!("'{}'", value.replace('\'', "''"))),
    }
}

fn execute_chain(chain: &str, global: &Global, input: Option<Data>) -> Result<Data> {
    let stages = parse_chain_stages(chain, global)?;
    if let Some(data) = execute_fused_record_chain(&stages, global, input.clone())? {
        return Ok(data);
    }
    let mut data = input;
    for stage in stages {
        data = Some(execute_command(&stage.command, &stage.global, data, true)?);
    }
    data.ok_or_else(|| PrismError::usage("empty chain"))
}

#[derive(Clone, Debug)]
struct ParsedStage {
    command: Command,
    global: Global,
}

fn parse_chain_stages(chain: &str, global: &Global) -> Result<Vec<ParsedStage>> {
    let stages = split_chain(chain)?;
    let mut parsed = Vec::with_capacity(stages.len());
    for (idx, stage) in stages.iter().enumerate() {
        let words = shell_words::split(stage)
            .map_err(|err| PrismError::usage(format!("invalid chain stage: {err}")))?;
        if words.is_empty() {
            return Err(PrismError::usage("empty chain stage"));
        }
        let mut argv = vec!["prism".to_string()];
        argv.extend(words);
        let cli = Cli::try_parse_from(argv).map_err(|err| PrismError::usage(err.to_string()))?;
        let stage_global = merge_stage_global(global, &cli.global);
        if idx > 0 && is_generator_command(&cli.command) {
            return Err(PrismError::usage(
                "a generator may only be the first chain stage",
            ));
        }
        if idx == 0 && global.count.is_some() && !is_generator_command(&cli.command) {
            return Err(PrismError::usage(
                "-n applies only when the first chain stage is a generator",
            ));
        }
        parsed.push(ParsedStage {
            command: cli.command,
            global: stage_global,
        });
    }
    Ok(parsed)
}

fn execute_fused_record_chain(
    stages: &[ParsedStage],
    global: &Global,
    input: Option<Data>,
) -> Result<Option<Data>> {
    if stages.is_empty() || stages.iter().any(|stage| stage.global.keep_going) {
        return Ok(None);
    }
    let mut transforms = Vec::with_capacity(stages.len());
    for (idx, stage) in stages.iter().enumerate() {
        if idx == 0 && is_generator_command(&stage.command) {
            return Ok(None);
        }
        let Some(transform) = record_transform(&stage.command)? else {
            return Ok(None);
        };
        transforms.push(transform);
    }

    let input_global = stages.first().map(|stage| &stage.global).unwrap_or(global);
    let mut out = Vec::new();
    for record in records_from_input(input_global, input, None)? {
        let mut value = record;
        for transform in &mut transforms {
            value = transform.apply(&value)?;
        }
        out.push(value);
    }
    Ok(Some(Data::Records(out)))
}

fn merge_stage_global(outer: &Global, stage: &Global) -> Global {
    let mut merged = outer.clone();
    if stage.count.is_some() {
        merged.count = stage.count;
    }
    if stage.seed.is_some() {
        merged.seed = stage.seed.clone();
    }
    merged.null |= stage.null;
    merged.no_newline |= stage.no_newline;
    merged.raw |= stage.raw;
    merged.json |= stage.json;
    merged.keep_going |= stage.keep_going;
    merged.quiet |= stage.quiet;
    merged
}

fn split_chain(chain: &str) -> Result<Vec<String>> {
    let mut stages = Vec::new();
    let mut current = String::new();
    let mut single = false;
    let mut double = false;
    let mut escape = false;
    for ch in chain.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '\\' if double => {
                current.push(ch);
                escape = true;
            }
            '\'' if !double => {
                single = !single;
                current.push(ch);
            }
            '"' if !single => {
                double = !double;
                current.push(ch);
            }
            '|' if !single && !double => {
                stages.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if single || double {
        return Err(PrismError::usage("unterminated quote in chain"));
    }
    if !current.trim().is_empty() {
        stages.push(current.trim().to_string());
    }
    Ok(stages)
}

fn is_generator_command(command: &Command) -> bool {
    matches!(
        command,
        Command::Dt(_) | Command::Rand(_) | Command::Seq(_) | Command::Repeat(_)
    ) || matches!(command, Command::Pad(args) if args.value.is_some())
}

fn execute_alias(args: &RunArgs, global: &Global, input: Option<Data>) -> Result<Data> {
    let config = load_config()?;
    let chain = config
        .alias
        .entries
        .get(&args.alias)
        .ok_or_else(|| PrismError::runtime(format!("alias not found: {}", args.alias)))?;
    let chain = inject_alias_args(chain, &args.args)?;
    execute_chain(&chain, global, input)
}

fn inject_alias_args(chain: &str, args: &[String]) -> Result<String> {
    if args.is_empty() {
        return Ok(chain.to_string());
    }
    let mut stages = split_chain(chain)?;
    let first = stages
        .first_mut()
        .ok_or_else(|| PrismError::usage("alias chain is empty"))?;
    for arg in args {
        first.push(' ');
        first.push_str(&quote_shell_for_chain(arg));
    }
    Ok(stages.join(" | "))
}

fn quote_shell_for_chain(value: &str) -> String {
    quote_shell(value)
}

fn command_alias(command: &AliasCommand) -> Result<String> {
    match command {
        AliasCommand::Path => Ok(config_path()?.display().to_string()),
        AliasCommand::List => {
            let config = load_config()?;
            Ok(config
                .alias
                .entries
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"))
        }
        AliasCommand::Show { name } => {
            let config = load_config()?;
            config
                .alias
                .entries
                .get(name)
                .cloned()
                .ok_or_else(|| PrismError::runtime(format!("alias not found: {name}")))
        }
        AliasCommand::Add { name, chain } => {
            let mut config = load_config().unwrap_or_default();
            config.alias.entries.insert(name.clone(), chain.clone());
            save_config(&config)?;
            Ok(name.clone())
        }
        AliasCommand::Rm { name } => {
            let mut config = load_config()?;
            config.alias.entries.remove(name);
            save_config(&config)?;
            Ok(name.clone())
        }
    }
}

fn config_path() -> Result<PathBuf> {
    if let Some(base) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(base).join("prism/config.toml"));
    }
    let dirs = directories::BaseDirs::new()
        .ok_or_else(|| PrismError::runtime("could not find config directory"))?;
    Ok(dirs.config_dir().join("prism/config.toml"))
}

fn load_config() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let content = fs::read_to_string(&path)?;
    toml::from_str(&content).map_err(|err| PrismError::runtime(format!("invalid config: {err}")))
}

fn save_config(config: &Config) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(config)
        .map_err(|err| PrismError::runtime(format!("could not serialize config: {err}")))?;
    fs::write(path, content)?;
    Ok(())
}

fn decode_escapes(value: &str) -> String {
    value
        .replace("\\t", "\t")
        .replace("\\n", "\n")
        .replace("\\0", "\0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_range_selects_to_last() {
        let args = FieldArgs {
            spec: "2..-1".to_string(),
            delimiter: None,
            regex: false,
            osep: ",".to_string(),
            strict_fields: false,
            value: None,
        };
        let selections = parse_field_spec(&args.spec).expect("field spec");
        let osep = decode_escapes(&args.osep);
        assert_eq!(
            field_record("a b c d", &args, &selections, &osep, None).expect("field"),
            "b,c,d"
        );
    }

    #[test]
    fn slug_normalizes_ascii() {
        assert_eq!(slugify("Hello, World!", "-", None, false), "hello-world");
    }

    #[test]
    fn seeded_rng_is_stable() {
        let global = Global {
            seed: Some("fixtures".to_string()),
            ..Global::default()
        };
        let mut one = rng_from_global(&global, "rand");
        let mut two = rng_from_global(&global, "rand");
        let first = random_from_charset(&mut one, "abc123", 20);
        let second = random_from_charset(&mut two, "abc123", 20);
        assert_eq!(first, second);
    }
}
