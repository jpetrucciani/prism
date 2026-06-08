#[test]
fn docs_name_the_full_v1_surface_without_stale_status() {
    let readme = include_str!("../README.md");
    let cli = include_str!("../docs/cli.md");
    let performance = include_str!("../docs/performance.md");
    let release = include_str!("../docs/release.md");
    let testing = include_str!("../docs/testing.md");

    assert!(!readme.contains("has not been scaffolded yet"));
    assert!(!readme.contains("planned static Rust CLI"));
    assert!(readme.contains("static Rust CLI"));

    for command in [
        "dt", "rand", "seq", "repeat", "pad", "case", "slug", "trim", "squeeze", "wrap", "indent",
        "dedent", "replace", "field", "slice", "lines", "enc", "hash", "tpl", "quote", "do",
        "alias", "run",
    ] {
        assert!(
            cli.contains(command),
            "docs/cli.md should mention {command}"
        );
    }

    for topic in [
        "Golden CLI tests",
        "Property tests",
        "Known-answer vectors",
        "Cross-platform tests",
        "Performance matrix",
        "Documentation tests",
        "Release acceptance checklist",
    ] {
        assert!(
            testing.contains(topic),
            "docs/testing.md should mention {topic}"
        );
    }

    for topic in [
        "PRISM_TZ",
        "TZ",
        "XDG_CONFIG_HOME",
        "prism version",
        "prism completions",
    ] {
        assert!(cli.contains(topic), "docs/cli.md should mention {topic}");
    }

    for topic in ["GitHub releases", "SHA256SUMS", "release_smoke.py"] {
        assert!(readme.contains(topic), "README.md should mention {topic}");
    }

    for topic in ["Automatic version tags", "Release artifacts", "SHA256SUMS"] {
        assert!(
            release.contains(topic),
            "docs/release.md should mention {topic}"
        );
    }

    for topic in ["perf_matrix", "dt", "rand", "hash", "tpl", "alias"] {
        assert!(
            performance.contains(topic),
            "docs/performance.md should mention {topic}"
        );
    }
}
