use std::env;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(zwo_sdk_stub)");
    println!("cargo:rerun-if-changed=src/ffi_stub.c");

    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_SDK_STUB");
    if env::var_os("CARGO_FEATURE_SDK_STUB").is_some() {
        println!("cargo:rustc-cfg=zwo_sdk_stub");
        cc::Build::new()
            .file("src/ffi_stub.c")
            .compile("ASICamera2");
    } else {
        println!("cargo:rustc-link-search=native=/usr/local/lib");
        println!("cargo:rustc-link-lib=dylib=ASICamera2");
    }
}
