// Re-embed the web UI whenever `ui/dist` changes (rust-embed reads it at compile time).
fn main() {
    println!("cargo:rerun-if-changed=../../ui/dist");
}
