fn main() {
    // macOS development builds embed the Dock icon in the executable.
    // Rebuild when artwork changes, even when no Rust source changed.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
