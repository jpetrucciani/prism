use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> String {
    env!("CARGO_BIN_EXE_prism").to_string()
}

fn run<I, S>(args: I, stdin: &[u8]) -> (i32, Vec<u8>, Vec<u8>)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new(bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn prism");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait prism");
    (
        output.status.code().unwrap_or(255),
        output.stdout,
        output.stderr,
    )
}

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("prism-{name}-{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn trim_maps_records_and_preserves_inner_empty_record() {
    let (code, stdout, stderr) = run(["trim"], b"  a  \n\n  b  \n");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"a\n\nb\n");
}

#[test]
fn null_record_separator_round_trips_records() {
    let (code, stdout, stderr) = run(["-0", "case", "upper"], b"a\0b\0");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"A\0B\0");
}

#[test]
fn field_uses_one_based_dotdot_ranges() {
    let (code, stdout, stderr) = run(["field", "2..-1", "--osep", ","], b"a b c d\n");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"b,c,d\n");
}

#[test]
fn enc_base64_round_trips_whole_input() {
    let (code, encoded, stderr) = run(["enc", "base64"], b"a\nb\n");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let encoded_text = String::from_utf8(encoded).expect("utf8");
    let (code, decoded, stderr) = run(["enc", "base64", "-d"], encoded_text.as_bytes());
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(decoded, b"a\nb\n");
}

#[test]
fn hash_sha256_known_answer() {
    let (code, stdout, stderr) = run(["hash", "sha256"], b"abc");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8(stdout).expect("utf8"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n"
    );
}

#[test]
fn seeded_rand_is_byte_identical_across_runs() {
    let (code, one, stderr) = run(["--seed", "fixtures", "rand", "--hex", "8"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let (code, two, stderr) = run(["--seed", "fixtures", "rand", "--hex", "8"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(one, two);
}

#[test]
fn chain_maps_records() {
    let (code, stdout, stderr) = run(["do", "trim | case snake | slug"], b"  Hello World  \n");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"hello-world\n");
}

#[test]
fn template_precedence_and_required_errors() {
    let (code, stdout, stderr) = run(["tpl", "--set", "PORT=8080"], b"port=${PORT:-80}\n");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"port=8080\n\n");

    let (code, _stdout, stderr) = run(["tpl"], b"${MISSING:?missing}\n");
    assert_eq!(code, 3);
    assert!(String::from_utf8_lossy(&stderr).contains("missing"));
}

#[test]
fn json_rendering_wraps_records() {
    let (code, stdout, stderr) = run(["--json", "field", "2"], b"a b\n");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"\"b\"\n");
}

#[test]
fn output_file_is_written() {
    let dir = temp_dir("out");
    let target = dir.join("nested/out.txt");
    let (code, stdout, stderr) = run(
        [
            "--mkdir",
            "-o",
            target.to_str().expect("path"),
            "case",
            "upper",
        ],
        b"abc\n",
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert!(stdout.is_empty());
    assert_eq!(fs::read(&target).expect("read output"), b"ABC\n");
}

#[test]
fn alias_run_executes_configured_chain() {
    let dir = temp_dir("alias");
    let config_home = dir.join("config");
    let config_dir = config_home.join("prism");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::write(
        config_dir.join("config.toml"),
        "[alias]\nsnakeslug = \"trim | case snake | slug\"\n",
    )
    .expect("write config");
    let mut child = Command::new(bin())
        .args(["run", "snakeslug"])
        .env("XDG_CONFIG_HOME", &config_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn prism");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"  Hello World  \n")
        .expect("stdin write");
    let output = child.wait_with_output().expect("wait");
    assert_eq!(
        output.status.code().unwrap_or(255),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"hello-world\n");
}

fn run_with_env<I, S>(args: I, stdin: &[u8], envs: &[(&str, &PathBuf)]) -> (i32, Vec<u8>, Vec<u8>)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(bin());
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("PRISM_TZ")
        .env_remove("TZ");
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn prism");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait prism");
    (
        output.status.code().unwrap_or(255),
        output.stdout,
        output.stderr,
    )
}

fn run_with_str_env<I, S>(args: I, stdin: &[u8], envs: &[(&str, &str)]) -> (i32, Vec<u8>, Vec<u8>)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(bin());
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn prism");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait prism");
    (
        output.status.code().unwrap_or(255),
        output.stdout,
        output.stderr,
    )
}

#[test]
fn dt_uses_prism_tz_then_tz_environment_defaults() {
    let (code, stdout, stderr) = run_with_str_env(
        ["dt", "--from", "0", "--fmt", "%F %H:%M %z"],
        b"",
        &[
            ("PRISM_TZ", "America/New_York"),
            ("TZ", "America/Los_Angeles"),
        ],
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"1969-12-31 19:00 -0500\n");

    let (code, stdout, stderr) = run_with_str_env(
        ["dt", "--from", "0", "--fmt", "%F %H:%M %z"],
        b"",
        &[("TZ", "America/Los_Angeles")],
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"1969-12-31 16:00 -0800\n");
}

#[test]
fn dt_explicit_timezone_wins_over_environment() {
    let (code, stdout, stderr) = run_with_str_env(
        [
            "dt",
            "--tz",
            "Etc/UTC",
            "--from",
            "0",
            "--fmt",
            "%F %H:%M %z",
        ],
        b"",
        &[("PRISM_TZ", "America/New_York")],
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"1970-01-01 00:00 +0000\n");
}

#[test]
fn dt_invalid_env_timezone_is_usage_error() {
    let (code, _stdout, stderr) =
        run_with_str_env(["dt", "--from", "0"], b"", &[("PRISM_TZ", "Not/AZone")]);
    assert_eq!(code, 2);
    assert!(String::from_utf8_lossy(&stderr).contains("invalid timezone from PRISM_TZ"));
}

#[test]
fn dt_formats_epoch_and_clamps_months() {
    let (code, stdout, stderr) = run(["dt", "--utc", "--from", "0", "--fmt", "%F"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"1970-01-01\n");

    let (code, stdout, stderr) = run(
        ["dt", "--utc", "--at", "2025-01-31", "+1mo", "--fmt", "%F"],
        b"",
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"2025-02-28\n");
}

#[test]
fn help_exits_successfully_and_documents_record_model() {
    let (code, stdout, stderr) = run(["--help"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert!(stderr.is_empty());
    let help = String::from_utf8(stdout).expect("utf8 help");
    assert!(help.contains("acquire input, split into records"));
    assert!(help.contains("does not read broad PRISM_* option overrides"));
    assert!(help.contains("PRISM_TZ, then TZ"));
    assert!(help.contains("XDG_CONFIG_HOME changes the config path"));
    assert!(help.contains("Config [defaults] currently supports seed"));
    assert!(help.contains("tpl verb reads the process environment"));
    assert!(help.contains("--null"));
    assert!(help.contains("version"));
}

#[test]
fn subcommand_help_documents_options() {
    let (code, stdout, stderr) = run(["rand", "--help"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert!(stderr.is_empty());
    let help = String::from_utf8(stdout).expect("utf8 help");
    assert!(help.contains("Generate an RFC 4122 version 4 UUID"));
    assert!(help.contains("--uuid7"));
    assert!(help.contains("--wordlist"));
}

#[test]
fn template_help_documents_environment_precedence() {
    let (code, stdout, stderr) = run(["tpl", "--help"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert!(stderr.is_empty());
    let help = String::from_utf8(stdout).expect("utf8 help");
    assert!(help.contains("Process environment values win"));
    assert!(help.contains("--env-file-override"));
    assert!(help.contains("--set"));
    assert!(help.contains("always wins over env and --env-file"));
}

#[test]
fn version_subcommand_embeds_cargo_version() {
    let (code, stdout, stderr) = run(["version"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let version = String::from_utf8(stdout).expect("utf8 version");
    assert!(version.starts_with(&format!("prism {}\n", env!("CARGO_PKG_VERSION"))));
    assert!(version.contains("build-profile:"));
    assert!(version.contains("build-commit:"));
    assert!(version.contains("rng-contract: prism-rng-v1"));
    assert!(version.contains("wordlist: builtin-demo-v1"));
}

#[test]
fn completions_generate_shell_output_without_config() {
    let (code, stdout, stderr) = run(["completions", "bash"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let completions = String::from_utf8(stdout).expect("utf8 completions");
    assert!(completions.contains("prism"));
    assert!(completions.contains("completions"));
}

#[test]
fn version_flag_is_conventional_one_line_cargo_version() {
    let (code, stdout, stderr) = run(["--version"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert!(stderr.is_empty());
    assert_eq!(
        String::from_utf8(stdout).expect("utf8 version"),
        format!("prism {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn rand_uuid_shapes_and_binary_bytes() {
    let (code, stdout, stderr) = run(["--seed", "fixtures", "rand", "--uuid"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let uuid = String::from_utf8(stdout).expect("uuid utf8");
    assert_eq!(uuid.trim().len(), 36);
    assert_eq!(uuid.trim().as_bytes()[14], b'4');

    let (code, stdout, stderr) = run(["--seed", "fixtures", "rand", "--bytes", "16"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout.len(), 16);
}

#[test]
fn generation_and_text_transform_matrix() {
    let cases: Vec<(Vec<&str>, &[u8], &[u8])> = vec![
        (
            vec!["seq", "1..3", "--fmt", "item-%03d"],
            b"",
            b"item-001\nitem-002\nitem-003\n",
        ),
        (vec!["repeat", "x", "3", "--sep", ","], b"", b"x,x,x\n"),
        (
            vec!["pad", "--left", "5", "--fill", "0", "42"],
            b"",
            b"00042\n",
        ),
        (vec!["case", "pascal", "hello_world"], b"", b"HelloWorld\n"),
        (
            vec!["slug", "--sep", "_", "Hello, World!"],
            b"",
            b"hello_world\n",
        ),
        (vec!["squeeze"], b"a   b\n", b"a b\n"),
        (
            vec!["replace", "--regex", "\\d+", "N"],
            b"a123b\n",
            b"aNb\n",
        ),
        (vec!["slice", "1..4"], b"hello\n", b"ell\n"),
        (vec!["lines", "--uniq-global"], b"a\nb\na\n", b"a\nb\n"),
        (vec!["indent", "--spaces", "2"], b"a\nb\n", b"  a\n  b\n"),
        (vec!["dedent"], b"  a\n  b\n", b"a\nb\n"),
        (
            vec!["wrap", "--width", "5"],
            b"one two three\n",
            b"one\ntwo\nthree\n",
        ),
    ];

    for (args, stdin, expected) in cases {
        let (code, stdout, stderr) = run(args.clone(), stdin);
        assert_eq!(
            code,
            0,
            "args {args:?}, stderr: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(stdout, expected, "args {args:?}");
    }
}

#[test]
fn encoding_codec_matrix_and_keep_going() {
    let cases: Vec<(Vec<&str>, &[u8], &[u8])> = vec![
        (vec!["enc", "hex"], b"abc", b"616263\n"),
        (vec!["enc", "hex", "-d"], b"616263\n", b"abc"),
        (vec!["enc", "url", "--component"], b"a b/c", b"a%20b%2Fc\n"),
        (vec!["enc", "json"], b"a\"b", b"\"a\\\"b\"\n"),
        (vec!["quote", "shell", "a b"], b"", b"'a b'\n"),
        (vec!["quote", "sql", "Bob's"], b"", b"'Bob''s'\n"),
    ];

    for (args, stdin, expected) in cases {
        let (code, stdout, stderr) = run(args.clone(), stdin);
        assert_eq!(
            code,
            0,
            "args {args:?}, stderr: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(stdout, expected, "args {args:?}");
    }

    let (code, stdout, stderr) = run(
        ["--keep-going", "enc", "base64", "-d", "--per-line"],
        b"YQ==\n!\nYg==\n",
    );
    assert_eq!(code, 1);
    assert_eq!(stdout, b"a\nb\n");
    assert!(String::from_utf8_lossy(&stderr).contains("record 2"));
}

#[test]
fn hash_modes_and_hmac_vectors() {
    let (code, stdout, stderr) = run(["hash", "sha512", "--short", "8"], b"abc");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"ddaf35a1\n");

    let (code, stdout, stderr) = run(
        ["hash", "hmac-sha256", "--key", "key"],
        b"The quick brown fox jumps over the lazy dog",
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        stdout,
        b"f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8\n"
    );

    let (code, stdout, stderr) = run(["hash", "sha256", "--per-line"], b"a\nb\n");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let text = String::from_utf8(stdout).expect("utf8");
    assert_eq!(text.lines().count(), 2);
}

#[test]
fn template_recursive_no_gen_and_env_file_precedence() {
    let dir = temp_dir("tpl");
    let env_file = dir.join("vars.env");
    fs::write(&env_file, "PORT=9000\n").expect("write env file");

    let (code, stdout, stderr) = run(
        ["tpl", "--recursive", "--set", "A=${B}", "--set", "B=ok"],
        b"${A}",
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"ok\n");

    let (code, _stdout, stderr) = run(["tpl", "--no-gen"], b"${@uuid}");
    assert_eq!(code, 2);
    assert!(String::from_utf8_lossy(&stderr).contains("disabled"));

    let (code, stdout, stderr) = run(
        ["tpl", "--env-file", env_file.to_str().expect("env path")],
        b"${PORT}",
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"9000\n");
}

#[test]
fn alias_management_and_bare_dispatch() {
    let dir = temp_dir("alias-management");
    let config_home = dir.join("config");

    let (code, stdout, stderr) = run_with_env(
        ["alias", "add", "loud", "case upper"],
        b"",
        &[("XDG_CONFIG_HOME", &config_home)],
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"loud\n");

    let (code, stdout, stderr) = run_with_env(
        ["alias", "show", "loud"],
        b"",
        &[("XDG_CONFIG_HOME", &config_home)],
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"case upper\n");

    let config_path = config_home.join("prism/config.toml");
    let mut config = fs::read_to_string(&config_path).expect("config");
    config = config.replace("expand_bare = false", "expand_bare = true");
    fs::write(&config_path, config).expect("rewrite config");

    let (code, stdout, stderr) =
        run_with_env(["loud"], b"abc\n", &[("XDG_CONFIG_HOME", &config_home)]);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"ABC\n");
}

#[test]
fn output_errors_append_and_flag_conflicts() {
    let dir = temp_dir("output-flags");
    let missing = dir.join("missing/out.txt");
    let (code, _stdout, stderr) = run(
        ["-o", missing.to_str().expect("path"), "case", "upper"],
        b"abc\n",
    );
    assert_eq!(code, 1);
    assert!(String::from_utf8_lossy(&stderr).contains("parent directory"));

    let target = dir.join("append.txt");
    fs::write(&target, b"old\n").expect("seed append file");
    let (code, stdout, stderr) = run(
        [
            "--append",
            "-o",
            target.to_str().expect("path"),
            "case",
            "upper",
        ],
        b"new\n",
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert!(stdout.is_empty());
    assert_eq!(fs::read(&target).expect("read append"), b"old\nNEW\n");

    let (code, _stdout, stderr) = run(["--raw", "--json", "case", "upper"], b"abc\n");
    assert_eq!(code, 2);
    assert!(String::from_utf8_lossy(&stderr).contains("mutually exclusive"));
}

#[test]
fn dt_handles_dst_gap_fold_and_invalid_timezone() {
    let (code, stdout, stderr) = run(
        [
            "dt",
            "--tz",
            "America/New_York",
            "--at",
            "2025-03-09T02:30:00",
            "--fmt",
            "%H:%M",
        ],
        b"",
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"03:30\n");

    let (code, stdout, stderr) = run(
        [
            "dt",
            "--tz",
            "America/New_York",
            "--at",
            "2025-11-02T01:30:00",
            "--fmt",
            "%z",
        ],
        b"",
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"-0400\n");

    let (code, _stdout, stderr) = run(["dt", "--tz", "Not/AZone", "--at", "2025-01-01"], b"");
    assert_eq!(code, 2);
    assert!(String::from_utf8_lossy(&stderr).contains("invalid timezone"));
}

#[test]
fn more_codecs_round_trip_or_match_known_outputs() {
    for codec in ["base32", "base32hex", "ascii85", "rot13", "csv-field"] {
        let (code, encoded, stderr) = run(["--no-newline", "enc", codec], b"hello, world");
        assert_eq!(
            code,
            0,
            "codec {codec}, stderr: {}",
            String::from_utf8_lossy(&stderr)
        );
        let (code, decoded, stderr) = run(["enc", codec, "-d"], &encoded);
        assert_eq!(
            code,
            0,
            "codec {codec}, stderr: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(decoded, b"hello, world", "codec {codec}");
    }

    let (code, stdout, stderr) = run(["--no-newline", "enc", "html"], b"<a&b>");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"&lt;a&amp;b&gt;");
    let (code, decoded, stderr) = run(["enc", "html", "-d"], &stdout);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(decoded, b"<a&b>");

    let (code, stdout, stderr) = run(["enc", "punycode"], "bücher.example".as_bytes());
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"xn--bcher-kva.example\n");
}

#[test]
fn hash_vectors_cover_more_algorithms_and_raw_mode() {
    let cases = [
        ("md5", "900150983cd24fb0d6963f7d28e17f72"),
        ("sha1", "a9993e364706816aba3e25717850c26c9cd0d89d"),
        (
            "sha3-256",
            "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532",
        ),
    ];
    for (algo, expected) in cases {
        let (code, stdout, stderr) = run(["hash", algo], b"abc");
        assert_eq!(
            code,
            0,
            "algo {algo}, stderr: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(String::from_utf8(stdout).expect("utf8").trim(), expected);
    }

    let (code, stdout, stderr) = run(["hash", "sha256", "--raw"], b"abc");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout.len(), 32);
}

#[test]
fn field_strict_slice_grapheme_lines_and_display_width_edges() {
    let (code, _stdout, stderr) = run(["field", "0"], b"a b\n");
    assert_eq!(code, 2);
    assert!(String::from_utf8_lossy(&stderr).contains("1-based"));

    let (code, _stdout, stderr) = run(["field", "3", "--strict-fields"], b"a b\n");
    assert_eq!(code, 1);
    assert!(String::from_utf8_lossy(&stderr).contains("out of range"));

    let (code, stdout, stderr) = run(["slice", "--graphemes", "0..1"], "🇺🇸a\n".as_bytes());
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, "🇺🇸\n".as_bytes());

    let (code, stdout, stderr) = run(["pad", "--right", "4", "中"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, "中  \n".as_bytes());

    let (code, stdout, stderr) = run(["lines", "--sort", "--numeric", "--reverse"], b"2\n10\n1\n");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"10\n2\n1\n");

    let (code, one, stderr) = run(["--seed", "shuffle", "lines", "--shuffle"], b"a\nb\nc\nd\n");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let (code, two, stderr) = run(["--seed", "shuffle", "lines", "--shuffle"], b"a\nb\nc\nd\n");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(one, two);
}

#[test]
fn template_cycle_and_config_default_seed_are_enforced() {
    let (code, _stdout, stderr) = run(["tpl", "--recursive", "--set", "A=${A}"], b"${A}");
    assert_eq!(code, 4);
    assert!(String::from_utf8_lossy(&stderr).contains("cycle"));

    let dir = temp_dir("defaults-seed");
    let config_home = dir.join("config");
    let config_dir = config_home.join("prism");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::write(
        config_dir.join("config.toml"),
        "[defaults]\nseed = \"fixtures\"\n",
    )
    .expect("write config");

    let (code, one, stderr) = run_with_env(
        ["rand", "--hex", "8"],
        b"",
        &[("XDG_CONFIG_HOME", &config_home)],
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let (code, two, stderr) = run_with_env(
        ["rand", "--hex", "8"],
        b"",
        &[("XDG_CONFIG_HOME", &config_home)],
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(one, two);
}

#[test]
fn every_documented_hash_algorithm_has_a_vector_or_shape_check() {
    let exact = [
        ("md5", "900150983cd24fb0d6963f7d28e17f72"),
        ("sha1", "a9993e364706816aba3e25717850c26c9cd0d89d"),
        ("sha224", "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7"),
        ("sha256", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        ("sha384", "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"),
        ("sha512", "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"),
        ("sha3-256", "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532"),
        ("sha3-512", "b751850b1a57168a5693cd924b6b096e08f621827444f70d884f5d0240d2712e10e116e9192af3c91a7ec57647e3934057340b4cf408d5a56592f8274eec53f0"),
        ("blake2b", "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923"),
        ("blake2s", "508c5e8c327c14e2e1a72ba34eeb452f37458b209ed63a294d999b4c86675982"),
        ("crc32", "352441c2"),
        ("blake3", "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"),
        ("fnv1a", "e71fa2190541574b"),
    ];
    for (algo, expected) in exact {
        let (code, stdout, stderr) = run(["hash", algo], b"abc");
        assert_eq!(
            code,
            0,
            "algo {algo}, stderr: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(String::from_utf8(stdout).expect("utf8").trim(), expected);
    }

    for (algo, expected) in [("xxh64", "ef46db3751d8e999"), ("xxh3", "2d06800538d394c2")] {
        let (code, stdout, stderr) = run(["hash", algo], b"");
        assert_eq!(
            code,
            0,
            "algo {algo}, stderr: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(String::from_utf8(stdout).expect("utf8").trim(), expected);
    }
}

#[test]
fn every_documented_codec_has_direct_coverage() {
    let cases: Vec<(Vec<&str>, &[u8], &[u8])> = vec![
        (vec!["--no-newline", "enc", "base64url"], b"hi?", b"aGk_"),
        (
            vec!["--no-newline", "enc", "base64url", "--no-pad"],
            b"hi",
            b"aGk",
        ),
        (
            vec!["--no-newline", "enc", "base16", "--upper"],
            b"abc",
            b"616263",
        ),
        (
            vec!["--no-newline", "enc", "xml"],
            b"<a&b>'\"",
            b"&lt;a&amp;b&gt;&apos;&quot;",
        ),
        (
            vec!["--no-newline", "enc", "quoted-printable"],
            b"hi there",
            b"hi there",
        ),
        (vec!["--no-newline", "enc", "shell"], b"a b", b"'a b'"),
    ];

    for (args, stdin, expected) in cases {
        let (code, stdout, stderr) = run(args.clone(), stdin);
        assert_eq!(
            code,
            0,
            "args {args:?}, stderr: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(stdout, expected, "args {args:?}");
    }

    let (code, _stdout, stderr) = run(["enc", "shell", "-d"], b"'a b'");
    assert_eq!(code, 2);
    assert!(String::from_utf8_lossy(&stderr).contains("encode-only"));
}

#[test]
fn readme_and_cli_examples_have_stable_smoke_coverage() {
    let cases: Vec<(Vec<&str>, &[u8], &[u8])> = vec![
        (
            vec!["dt", "--utc", "--from", "0", "--fmt", "%Y-%m-%dT%H:%M:%SZ"],
            b"",
            b"1970-01-01T00:00:00Z\n",
        ),
        (
            vec![
                "dt",
                "--tz",
                "America/New_York",
                "--at",
                "2025-01-01",
                "+1mo",
                "--fmt",
                "%F",
            ],
            b"",
            b"2025-02-01\n",
        ),
        (vec!["--seed", "fixtures", "rand", "--hex", "16"], b"", b""),
        (
            vec!["seq", "1..5", "--fmt", "item-%03d"],
            b"",
            b"item-001\nitem-002\nitem-003\nitem-004\nitem-005\n",
        ),
        (
            vec!["do", "trim | case snake | slug"],
            b"  Hello World  \n",
            b"hello-world\n",
        ),
        (vec!["field", "2"], b"a b c\n", b"b\n"),
        (vec!["lines", "--uniq-global"], b"a\nb\na\n", b"a\nb\n"),
        (
            vec!["tpl", "--set", "PORT=8080"],
            b"port=${PORT}\n",
            b"port=8080\n\n",
        ),
    ];

    for (args, stdin, expected) in cases {
        let (code, stdout, stderr) = run(args.clone(), stdin);
        assert_eq!(
            code,
            0,
            "args {args:?}, stderr: {}",
            String::from_utf8_lossy(&stderr)
        );
        if expected.is_empty() {
            assert!(!stdout.is_empty(), "args {args:?} should produce output");
        } else {
            assert_eq!(stdout, expected, "args {args:?}");
        }
    }
}

#[test]
fn version_and_standard_base64_no_pad_match_spec() {
    let (code, stdout, stderr) = run(["version"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let version = String::from_utf8(stdout).expect("version utf8");
    assert!(version.contains("rng-contract: prism-rng-v1"));
    assert!(version.contains("wordlist: builtin-demo-v1"));
    assert!(version.contains("target:"));
    assert!(version.contains("build-profile:"));
    assert!(version.contains("build-commit:"));

    let (code, stdout, stderr) = run(["--no-newline", "enc", "base64", "--no-pad"], b"hi?");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"aGk/");

    let (code, stdout, stderr) = run(["enc", "base64", "-d", "--no-pad"], b"aGk/");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, b"hi?");
}

#[test]
fn byte_slicing_template_helpers_config_defaults_and_stage_globals() {
    let (code, stdout, stderr) = run(["slice", "--bytes", "0..2"], "éa\n".as_bytes());
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, "é\n".as_bytes());

    let (code, _stdout, stderr) = run(["slice", "--bytes", "0..1"], "éa\n".as_bytes());
    assert_eq!(code, 1);
    assert!(String::from_utf8_lossy(&stderr).contains("UTF-8 boundaries"));

    let (code, stdout, stderr) = run(
        ["tpl", "--set", "PROJECT=My Feature", "--seed", "fixtures"],
        b"${@slug:${PROJECT}} ${@uuid7} ${@ulid} ${@rand:words:2}",
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let text = String::from_utf8(stdout).expect("template utf8");
    let parts: Vec<&str> = text.split_whitespace().collect();
    assert_eq!(parts[0], "my-feature");
    assert_eq!(parts[1].len(), 36);
    assert_eq!(parts[1].as_bytes()[14], b'7');
    assert_eq!(parts[2].len(), 26);
    assert_eq!(
        parts.len(),
        5,
        "two rand words should produce two fields: {text}"
    );

    let dir = temp_dir("config-defaults");
    let config_home = dir.join("config");
    let config_dir = config_home.join("prism");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::write(
        config_dir.join("config.toml"),
        "[defaults]\nno_newline = true\ncount = 2\nseed = \"fixtures\"\n",
    )
    .expect("write config");
    let (code, stdout, stderr) = run_with_env(
        ["rand", "--hex", "2"],
        b"",
        &[("XDG_CONFIG_HOME", &config_home)],
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let text = String::from_utf8(stdout).expect("rand utf8");
    assert_eq!(text.lines().count(), 2);
    assert!(!text.ends_with('\n'));

    let (code, one, stderr) = run(["do", "rand --hex 4 --seed fixtures | case upper"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let (code, two, stderr) = run(["do", "rand --hex 4 --seed fixtures | case upper"], b"");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert_eq!(one, two);
}

#[test]
fn file_output_is_all_or_nothing_and_preserves_mode() {
    let dir = temp_dir("atomic-output");
    let target = dir.join("out.txt");
    fs::write(&target, b"old\n").expect("seed output file");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&target).expect("metadata").permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&target, permissions).expect("set permissions");
    }

    let (code, stdout, stderr) = run(
        ["-o", target.to_str().expect("path"), "enc", "base64", "-d"],
        b"not base64!",
    );
    assert_eq!(code, 1);
    assert!(stdout.is_empty());
    assert!(String::from_utf8_lossy(&stderr).contains("decode failed"));
    assert_eq!(fs::read(&target).expect("read target"), b"old\n");

    let (code, stdout, stderr) = run(
        [
            "--keep-going",
            "-o",
            target.to_str().expect("path"),
            "enc",
            "base64",
            "-d",
            "--per-line",
        ],
        b"YQ==\n!\nYg==\n",
    );
    assert_eq!(code, 1);
    assert!(stdout.is_empty());
    assert!(String::from_utf8_lossy(&stderr).contains("one or more records failed"));
    assert_eq!(fs::read(&target).expect("read target"), b"old\n");

    let (code, stdout, stderr) = run(
        ["-o", target.to_str().expect("path"), "case", "upper"],
        b"new\n",
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    assert!(stdout.is_empty());
    assert_eq!(fs::read(&target).expect("read target"), b"NEW\n");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&target)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn binary_safe_codecs_round_trip_over_corpus() {
    let inputs: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"a".to_vec(),
        b"abc".to_vec(),
        b"hello\nworld".to_vec(),
        (0_u8..=31).collect(),
        (0_u8..=255).collect(),
    ];
    for codec in [
        "base64",
        "base64url",
        "base32",
        "base32hex",
        "hex",
        "ascii85",
    ] {
        for input in &inputs {
            let (code, encoded, stderr) = run(["--no-newline", "enc", codec], input);
            assert_eq!(
                code,
                0,
                "codec {codec}, input len {}, stderr: {}",
                input.len(),
                String::from_utf8_lossy(&stderr)
            );
            let (code, decoded, stderr) = run(["enc", codec, "-d"], &encoded);
            assert_eq!(
                code,
                0,
                "codec {codec}, input len {}, stderr: {}",
                input.len(),
                String::from_utf8_lossy(&stderr)
            );
            assert_eq!(&decoded, input, "codec {codec}, input len {}", input.len());
        }
    }
}

#[test]
fn codec_and_hash_modifiers_cover_documented_combinations() {
    let cases: Vec<(Vec<&str>, &[u8], &[u8])> = vec![
        (vec!["enc", "hex", "--upper"], b"abc", b"616263\n"),
        (vec!["enc", "base64", "--no-pad"], b"hi", b"aGk\n"),
        (vec!["enc", "base64url", "--no-pad"], b"hi?", b"aGk_\n"),
        (vec!["enc", "url"], b"a b", b"a%20b\n"),
        (vec!["enc", "url", "--component"], b"a/b", b"a%2Fb\n"),
        (
            vec!["hash", "sha256", "--upper", "--short", "8"],
            b"abc",
            b"BA7816BF\n",
        ),
        (
            vec!["hash", "sha256", "--base64"],
            b"abc",
            b"ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=\n",
        ),
    ];

    for (args, stdin, expected) in cases {
        let (code, stdout, stderr) = run(args.clone(), stdin);
        assert_eq!(
            code,
            0,
            "args {args:?}, stderr: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(stdout, expected, "args {args:?}");
    }
}

#[test]
fn template_now_offset_helper_is_supported() {
    let (code, stdout, stderr) = run(["tpl"], b"${@now:+1d:%F}");
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let text = String::from_utf8(stdout).expect("utf8");
    let value = text.trim();
    assert_eq!(value.len(), 10);
    assert_eq!(&value[4..5], "-");
    assert_eq!(&value[7..8], "-");
}

#[test]
fn seeded_fixture_outputs_are_exact_contract_vectors() {
    let cases: Vec<(Vec<&str>, &[u8])> = vec![
        (
            vec!["--seed", "fixtures", "rand", "--hex", "8"],
            b"7b118a56240ddfad\n",
        ),
        (
            vec!["--seed", "fixtures", "rand", "--uuid"],
            b"7b118a56-240d-4fad-975c-1ade6afdf900\n",
        ),
        (
            vec!["--seed", "fixtures", "rand", "--uuid7"],
            b"0050520d-d1cf-7b11-8a56-240ddfad575c\n",
        ),
        (
            vec!["--seed", "fixtures", "rand", "--ulid"],
            b"00A190VMEFFC8RMNH41QFTTNTW\n",
        ),
    ];

    for (args, expected) in cases {
        let (code, stdout, stderr) = run(args.clone(), b"");
        assert_eq!(
            code,
            0,
            "args {args:?}, stderr: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(stdout, expected, "args {args:?}");
    }
}

#[test]
fn binary_safe_codecs_round_trip_deterministic_fuzz_corpus() {
    let codecs = [
        "base64",
        "base64url",
        "base32",
        "base32hex",
        "hex",
        "ascii85",
    ];
    let mut state = 0x1234_5678_9abc_def0_u64;
    for case_idx in 0..96_usize {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let len = ((state >> 32) as usize) % 257;
        let mut input = Vec::with_capacity(len);
        for byte_idx in 0..len {
            state = state
                .wrapping_mul(2862933555777941757)
                .wrapping_add(3037000493);
            input.push(((state >> ((byte_idx % 8) * 8)) & 0xff) as u8);
        }
        for codec in codecs {
            let (code, encoded, stderr) = run(["--no-newline", "enc", codec], &input);
            assert_eq!(
                code,
                0,
                "codec {codec}, fuzz case {case_idx}, stderr: {}",
                String::from_utf8_lossy(&stderr)
            );
            let (code, decoded, stderr) = run(["enc", codec, "-d"], &encoded);
            assert_eq!(
                code,
                0,
                "codec {codec}, fuzz case {case_idx}, stderr: {}",
                String::from_utf8_lossy(&stderr)
            );
            assert_eq!(decoded, input, "codec {codec}, fuzz case {case_idx}");
        }
    }
}

#[test]
fn config_defaults_cover_supported_global_defaults() {
    let dir = temp_dir("config-defaults-all");
    let config_home = dir.join("config");
    let config_dir = config_home.join("prism");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::write(
        config_dir.join("config.toml"),
        "[defaults]\nseed = \"fixtures\"\ncount = 2\nno_newline = true\njson = true\n",
    )
    .expect("write config");

    let (code, stdout, stderr) = run_with_env(
        ["rand", "--hex", "2"],
        b"",
        &[("XDG_CONFIG_HOME", &config_home)],
    );
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let text = String::from_utf8(stdout).expect("utf8");
    assert_eq!(text.lines().count(), 2);
    assert!(text
        .lines()
        .all(|line| line.starts_with('"') && line.ends_with('"')));
    assert!(!text.ends_with('\n'));
}
