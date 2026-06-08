use std::io::Write;
use std::process::{Command, Stdio};

fn bin() -> String {
    env!("CARGO_BIN_EXE_prism").to_string()
}

#[derive(Debug)]
struct Example {
    args: Vec<String>,
    stdin: Vec<u8>,
    stdout: Vec<u8>,
}

#[test]
fn executable_docs_examples_match_output() {
    let examples = parse_examples(include_str!("../docs/examples.md"));
    assert!(
        !examples.is_empty(),
        "docs/examples.md should contain prism examples"
    );
    for example in examples {
        let mut child = Command::new(bin())
            .args(&example.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn prism");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(&example.stdin)
            .expect("write stdin");
        let output = child.wait_with_output().expect("wait prism");
        assert_eq!(
            output.status.code().unwrap_or(255),
            0,
            "args {:?}, stderr: {}",
            example.args,
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, example.stdout, "args {:?}", example.args);
    }
}

fn parse_examples(markdown: &str) -> Vec<Example> {
    let mut examples = Vec::new();
    let mut in_block = false;
    let mut args = Vec::new();
    let mut stdin = Vec::new();
    let mut stdout = Vec::new();
    for line in markdown.lines() {
        if line == "```prism-example" {
            in_block = true;
            args.clear();
            stdin.clear();
            stdout.clear();
            continue;
        }
        if in_block && line == "```" {
            examples.push(Example {
                args: args.clone(),
                stdin: stdin.clone(),
                stdout: stdout.clone(),
            });
            in_block = false;
            continue;
        }
        if !in_block {
            continue;
        }
        if let Some(command) = line.strip_prefix("$ prism ") {
            args = shell_words::split(command).expect("parse example command");
        } else if let Some(input) = line.strip_prefix("< ") {
            stdin.extend_from_slice(input.as_bytes());
            stdin.push(b'\n');
        } else if let Some(output) = line.strip_prefix("> ") {
            stdout.extend_from_slice(output.as_bytes());
            stdout.push(b'\n');
        }
    }
    examples
}
