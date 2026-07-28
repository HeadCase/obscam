fn main() {
    println!("cargo:rustc-link-search=native=/usr/local/lib");
    println!("cargo:rustc-link-lib=dylib=ASICamera2");
    println!("cargo:rerun-if-changed=build.rs");
}
