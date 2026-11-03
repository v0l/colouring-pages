use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(local_speech)");
    if std::env::var_os("CARGO_FEATURE_LOCAL_SPEECH").is_some()
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android")
    {
        println!("cargo:rustc-cfg=local_speech");
    }
    println!("cargo:rerun-if-env-changed=COLOURING_DEFAULTS");
    println!("cargo:rerun-if-changed=defaults.json");
    let source = std::env::var_os("COLOURING_DEFAULTS").map(PathBuf::from).unwrap_or_else(|| "defaults.json".into());
    let text = std::fs::read_to_string(&source).unwrap_or_else(|_| "{}".into());
    serde_json::from_str::<serde_json::Value>(&text).expect("defaults file is not valid JSON");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("defaults.json");
    std::fs::write(out, text).unwrap();
}
