//! Bundle the web client (`example_client/`) so `main.rs` can embed it and
//! serve it on the websocket port.  Needs node + npm; `npm install` is run
//! on first use.

use std::path::Path;
use std::process::Command;

fn main() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let client = repo.join("client");
    let example = repo.join("example_client");

    println!("cargo:rerun-if-changed={}", example.join("index.html").display());
    println!("cargo:rerun-if-changed={}", example.join("src").display());
    println!("cargo:rerun-if-changed={}", example.join("package.json").display());
    println!("cargo:rerun-if-changed={}", client.join("src").display());
    println!("cargo:rerun-if-changed={}", client.join("package.json").display());

    for dir in [&client, &example] {
        if !dir.join("node_modules").exists() {
            run(dir, &["install"]);
        }
    }
    run(&example, &["run", "build"]);
}

fn run(dir: &Path, args: &[&str]) {
    let status = Command::new("npm")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "bevy_pbr_game embeds the web client, which needs `npm` (node.js) \
                 to bundle: failed to run `npm {}` in {}: {e}",
                args.join(" "),
                dir.display()
            )
        });
    assert!(
        status.success(),
        "`npm {}` in {} failed ({status})",
        args.join(" "),
        dir.display()
    );
}
