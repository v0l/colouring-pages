fn main() {
    println!("cargo:rustc-check-cfg=cfg(local_speech)");
    if std::env::var_os("CARGO_FEATURE_LOCAL_SPEECH").is_some()
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android")
    {
        println!("cargo:rustc-cfg=local_speech");
    }
}
