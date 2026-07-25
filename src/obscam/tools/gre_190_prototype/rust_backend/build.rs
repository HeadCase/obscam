use std::env;

fn main() {
    let library_directory =
        env::var("ASI_SDK_LIBRARY_DIR").unwrap_or_else(|_| "/usr/local/lib".to_owned());
    println!("cargo:rustc-link-search=native={library_directory}");
    println!("cargo:rustc-link-lib=dylib=ASICamera2");
}
