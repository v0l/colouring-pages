#[cfg(not(target_os = "android"))]
fn main() -> std::process::ExitCode {
    match colouring_pages::run_native() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("colouring-pages: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "android")]
fn main() {}
