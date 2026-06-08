use std::env;

fn main() {
    let profile = env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=PRISM_BUILD_PROFILE={profile}");

    let commit = env::var("GITHUB_SHA")
        .or_else(|_| env::var("PRISM_BUILD_COMMIT"))
        .unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=PRISM_BUILD_COMMIT={commit}");
}
