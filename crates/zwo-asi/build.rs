fn main() {
    println!("cargo:rerun-if-env-changed=OBSCAM_ZWO_SDK_STUB");
    if std::env::var_os("OBSCAM_ZWO_SDK_STUB").is_some() {
        cc::Build::new()
            .file("src/ffi_stub.c")
            .compile("ASICamera2");
    } else {
        println!("cargo:rustc-link-search=native=/usr/local/lib");
        println!("cargo:rustc-link-lib=dylib=ASICamera2");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/ffi_stub.c");
}
