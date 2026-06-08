{ pkgs ? import
    (fetchTarball {
      name = "jpetrucciani-2026-05-28";
      url = "https://github.com/jpetrucciani/nix/archive/d5878c16ec3972f733251d1c113ee14bd07e0bd4.tar.gz";
      sha256 = "0jim9w50r3as7w45d0rr2msayqjyvgk0ij2fm206vzkh8ny18s6p";
    })
    { overlays = [ rustOverlay ]; }
, rustOverlay ? import
    (fetchTarball {
      name = "oxalica-2026-05-28";
      url = "https://github.com/oxalica/rust-overlay/archive/02f536e36eaee387594ce2a02d90ff678d056e0f.tar.gz";
      sha256 = "05hsspb54cmmljl0i0456gfda4w0wsa03f498bjjv1xbj28iw4lj";
    })
}:
let
  name = "prism";
  muslTarget = "x86_64-unknown-linux-musl";

  rust = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default.override {
    extensions = [ "rust-src" "rustc-dev" "rust-analyzer" ];
    targets = [
      muslTarget
      "aarch64-unknown-linux-musl"
      "x86_64-pc-windows-gnu"
    ];
  });

  rustPlatform = pkgs.makeRustPlatform {
    cargo = rust;
    rustc = rust;
  };

  mingw = pkgs.pkgsCross.mingwW64;

  scripts = with pkgs; {
    fmt = writers.writeBashBin "fmt" ''
      set -euo pipefail
      cargo fmt
    '';

    clippy_all = writers.writeBashBin "clippy_all" ''
      set -euo pipefail
      cargo clippy --all --benches --tests --examples --all-features -- -D warnings
    '';

    test_all_features = writers.writeBashBin "test_all_features" ''
      set -euo pipefail
      cargo test --all-features
    '';

    test_no_default_features = writers.writeBashBin "test_no_default_features" ''
      set -euo pipefail
      cargo test --no-default-features
    '';

    quality = writers.writeBashBin "quality" ''
      set -euo pipefail
      cargo fmt --check
      cargo clippy --all --benches --tests --examples --all-features -- -D warnings
      cargo test --all-features
      cargo test --no-default-features
    '';

    docs_build = writers.writeBashBin "docs_build" ''
      set -euo pipefail
      (
        cd docs
        bun install --frozen-lockfile
        bun run docs:build
      )
    '';

    build_static = writers.writeBashBin "build_static" ''
      set -euo pipefail
      RUSTFLAGS="''${RUSTFLAGS:-} -A linker-messages" \
        cargo zigbuild --release --locked --all-features --target ${muslTarget}
    '';

    perf_matrix = writers.writeBashBin "perf_matrix" ''
      set -euo pipefail
      python3 scripts/perf_matrix.py --build "$@"
    '';

    release_smoke = writers.writeBashBin "release_smoke" ''
      set -euo pipefail
      python3 scripts/release_smoke.py "$@"
    '';
  };

  packages = with pkgs; [
    bun
    cargo-zigbuild
    jq
    jfmt
    pkg-config
    python314
    rust
    yq-go
    zig
    mingw.stdenv.cc
    # mingw.windows.pthreads
  ] ++ builtins.attrValues scripts;

  shell = pkgs.mkShellNoCC {
    inherit name packages;
    RUST_SRC_PATH = "${rust}/lib/rustlib/src/rust/library";
  };

  bin = rustPlatform.buildRustPackage {
    pname = name;
    version = "0.0.0";
    src = pkgs.hax.filterSrc { path = ./.; };
    cargoLock.lockFile = ./Cargo.lock;
    auditable = false;
    strictDeps = true;
    nativeBuildInputs = with pkgs; [
      cargo-zigbuild
      pkg-config
      zig
    ];
    buildPhase = ''
      export HOME="$(mktemp -d)"
      RUSTFLAGS="''${RUSTFLAGS:-} -A linker-messages" \
        cargo zigbuild --release --locked --all-features --target ${muslTarget}
    '';
    installPhase = ''
      mkdir -p "$out/bin"
      cp "target/${muslTarget}/release/${name}" "$out/bin/${name}"
    '';
    meta.mainProgram = name;
  };
in
(shell.overrideAttrs (_: { inherit name; })) // {
  inherit bin scripts;
}
