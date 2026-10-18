fn main() {
    if std::env::var_os("CARGO_FEATURE_GUI").is_some() {
        println!("cargo:rustc-link-lib=fontconfig");
    }
}
