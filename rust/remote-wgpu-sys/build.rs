use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo = manifest.join("../..").canonicalize().unwrap();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // Generate the protobuf-c wire protocol code from the shared schema.
    let proto_gen = out.join("proto_gen");
    std::fs::create_dir_all(&proto_gen).unwrap();
    let status = Command::new("protoc")
        .arg(format!("--c_out={}", proto_gen.display()))
        .arg("-I")
        .arg(repo.join("proto"))
        .arg(repo.join("proto/remote_webgpu.proto"))
        .status()
        .expect("failed to run protoc (is it installed?)");
    assert!(status.success(), "protoc failed");

    // The wire protocol version lives in the schema; lift it into a Rust
    // constant so nothing has to hard-code it and drift.
    let schema = std::fs::read_to_string(repo.join("proto/remote_webgpu.proto")).unwrap();
    let version = schema
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name.trim() == "PROTOCOL_VERSION_CURRENT")
                .then(|| value.trim().trim_end_matches(';').trim())
        })
        .expect("PROTOCOL_VERSION_CURRENT missing from remote_webgpu.proto");
    std::fs::write(
        out.join("protocol_version.rs"),
        format!(
            "/// `PROTOCOL_VERSION_CURRENT` from `proto/remote_webgpu.proto`.\n             pub const PROTOCOL_VERSION: u32 = {version};\n"
        ),
    )
    .unwrap();

    let protobuf_c = pkg_config::Config::new()
        .probe("libprotobuf-c")
        .expect("libprotobuf-c not found via pkg-config");

    let lib_dir = repo.join("remote_webgpu");
    let mut build = cc::Build::new();
    build
        .file(lib_dir.join("src/remote_webgpu.c"))
        .file(lib_dir.join("src/remote_methods.c"))
        .file(lib_dir.join("src/stubs.c"))
        .file(proto_gen.join("remote_webgpu.pb-c.c"))
        .file(manifest.join("src/struct_sizes.c"))
        .include(lib_dir.join("include"))
        .include(lib_dir.join("src"))
        .include(&proto_gen);
    for path in &protobuf_c.include_paths {
        build.include(path);
    }
    build.compile("remote_webgpu");

    println!("cargo:rerun-if-changed={}", lib_dir.join("src").display());
    println!("cargo:rerun-if-changed={}", lib_dir.join("include").display());
    println!(
        "cargo:rerun-if-changed={}",
        repo.join("proto/remote_webgpu.proto").display()
    );
}
