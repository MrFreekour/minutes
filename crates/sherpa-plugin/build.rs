//! Make the plugin able to find its own sherpa libraries wherever it is put.
//!
//! On Linux, sherpa-rs links `libsherpa-onnx-c-api.so` and `libonnxruntime.so`
//! dynamically, and those land in this crate's `target/` directory. Without a
//! runpath the plugin only resolves them while it sits there, so copying it
//! anywhere else, which is exactly what installing and packaging both do,
//! produces a library that fails to load with a missing-dependency error.
//!
//! `$ORIGIN` points the loader at the plugin's own directory rather than the
//! build tree, so the plugin works anywhere its sibling libraries travel with
//! it. That is the shape the release archive already ships and the shape
//! docs/architecture/sherpa-engine.md tells people to install by hand.
//!
//! macOS links sherpa-onnx statically here, so there is no external library to
//! find and nothing to do. Windows resolves a DLL's dependencies from the
//! loading process's search path rather than the DLL's own directory, so it
//! needs different handling entirely; that is part of #645.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
    }
}
