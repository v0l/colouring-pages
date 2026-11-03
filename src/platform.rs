use std::path::Path;

#[cfg(target_os = "android")]
mod android {
    use jni::objects::JObject;
    use jni::{Env, JavaVM, jni_sig, jni_str};

    const PERMISSION: &str = "android.permission.RECORD_AUDIO";

    static ACTIVITY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    pub fn remember_activity(raw: *mut std::ffi::c_void) {
        ACTIVITY.store(raw as usize, std::sync::atomic::Ordering::Release);
    }

    pub fn with_activity<R>(
        f: impl for<'j> FnOnce(&mut Env<'j>, &JObject<'j>) -> jni::errors::Result<R>,
    ) -> Result<R, String> {
        let ctx = ndk_context::android_context();
        let raw = ACTIVITY.load(std::sync::atomic::Ordering::Acquire) as jni::sys::jobject;
        if ctx.vm().is_null() || raw.is_null() {
            return Err("no Android activity yet".into());
        }
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env: &mut Env<'_>| {
            let activity = unsafe { JObject::from_raw(env, raw) };
            f(env, &activity)
        })
        .map_err(|e| e.to_string())
    }

    pub fn microphone_allowed() -> bool {
        with_activity(|env, activity| {
            let name = env.new_string(PERMISSION)?;
            env.call_method(
                activity,
                jni_str!("checkSelfPermission"),
                jni_sig!("(Ljava/lang/String;)I"),
                &[(&name).into()],
            )?
            .i()
        })
        .is_ok_and(|r| r == 0)
    }

    pub fn ask_for_microphone() -> Result<(), String> {
        with_activity(|env, activity| {
            let string = env.find_class(jni_str!("java/lang/String"))?;
            let name = env.new_string(PERMISSION)?;
            let list = env.new_object_array(1, &string, &name)?;
            env.call_method(
                activity,
                jni_str!("requestPermissions"),
                jni_sig!("([Ljava/lang/String;I)V"),
                &[(&list).into(), 1.into()],
            )?;
            Ok(())
        })
    }

    pub fn print_pdf(path: &str) -> Result<String, String> {
        with_activity(|env, activity| {
            let path = env.new_string(path)?;
            env.call_method(
                activity,
                jni_str!("printPdf"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[(&path).into()],
            )?;
            Ok("the print dialog".to_string())
        })
    }

    pub fn save_download(path: &str, name: &str, mime: &str) -> Result<(), String> {
        with_activity(|env, activity| {
            let path = env.new_string(path)?;
            let name = env.new_string(name)?;
            let mime = env.new_string(mime)?;
            env.call_method(
                activity,
                jni_str!("saveDownload"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Z"),
                &[(&path).into(), (&name).into(), (&mime).into()],
            )?
            .z()
            .map(|_| ())
        })
    }
}

#[cfg(target_os = "android")]
pub use android::{
    with_activity, ask_for_microphone, microphone_allowed, print_pdf, remember_activity, save_download,
};

#[cfg(not(target_os = "android"))]
pub fn microphone_allowed() -> bool {
    true
}

#[cfg(not(target_os = "android"))]
pub fn ask_for_microphone() -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "android"))]
pub fn print_pdf(path: &str) -> Result<String, String> {
    let printer = pick_printer(&lpstat("-d"), &lpstat("-a"));
    print_to("lp", printer.as_deref(), Path::new(path))?;
    Ok(printer.unwrap_or_else(|| "the default printer".into()))
}

#[cfg(not(target_os = "android"))]
fn lpstat(flag: &str) -> String {
    std::process::Command::new("lpstat")
        .arg(flag)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

pub fn pick_printer(default_out: &str, accepting_out: &str) -> Option<String> {
    if let Some(name) = default_out.split("destination:").nth(1) {
        let name = name.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    accepting_out
        .lines()
        .filter(|l| l.contains("accepting requests") && !l.contains("not accepting"))
        .filter_map(|l| l.split_whitespace().next())
        .next()
        .map(str::to_string)
}

pub fn print_with(program: &str, file: &Path) -> Result<(), String> {
    print_to(program, None, file)
}

pub fn print_to(program: &str, printer: Option<&str>, file: &Path) -> Result<(), String> {
    let mut cmd = std::process::Command::new(program);
    if let Some(p) = printer {
        cmd.arg("-d").arg(p);
    }
    let out = cmd.arg(file).output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let said = String::from_utf8_lossy(&o.stderr).trim().to_string();
            Err(if said.is_empty() { format!("{program} could not print: {}", o.status) } else { said })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(format!("no printer program ({program}) found, save the PDF and print it from there"))
        }
        Err(e) => Err(format!("{program}: {e}")),
    }
}

#[cfg(not(target_os = "android"))]
pub fn save_download(path: &str, name: &str, _mime: &str) -> Result<(), String> {
    let dir = crate::outputs::export_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::copy(path, dir.join(name)).map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn alone() -> std::sync::MutexGuard<'static, ()> {
        ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn script(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().to_string()
    }

    #[test]
    fn printing_hands_the_file_to_the_print_program() {
        let _alone = alone();
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log");
        let lp = script(dir.path(), "lp", &format!("echo \"$@\" > {}", log.display()));
        print_with(&lp, Path::new("/tmp/x.pdf")).unwrap();
        assert_eq!(std::fs::read_to_string(log).unwrap().trim(), "/tmp/x.pdf");
    }

    #[test]
    fn a_named_printer_is_passed_to_lp() {
        let _alone = alone();
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log");
        let lp = script(dir.path(), "lp", &format!("echo \"$@\" > {}", log.display()));
        print_to(&lp, Some("Brother_X"), Path::new("/tmp/x.pdf")).unwrap();
        assert_eq!(std::fs::read_to_string(log).unwrap().trim(), "-d Brother_X /tmp/x.pdf");
    }

    #[test]
    fn the_default_printer_wins_and_otherwise_the_first_accepting_one() {
        let _alone = alone();
        let accepting = "Brother_DCP accepting requests since Mon 02 Nov\nOffice_Jet accepting requests since Mon\n";
        assert_eq!(pick_printer("system default destination: Office_Jet\n", accepting).as_deref(), Some("Office_Jet"));
        assert_eq!(pick_printer("no system default destination\n", accepting).as_deref(), Some("Brother_DCP"));
        let refusing = "Old_One not accepting requests since Mon\nBrother_DCP accepting requests since Mon\n";
        assert_eq!(pick_printer("no system default destination", refusing).as_deref(), Some("Brother_DCP"));
        assert_eq!(pick_printer("no system default destination", ""), None);
    }

    #[test]
    fn a_printer_that_fails_gives_its_own_words() {
        let _alone = alone();
        let dir = tempfile::tempdir().unwrap();
        let lp = script(dir.path(), "lp", "echo 'no default destination' >&2; exit 1");
        assert_eq!(print_with(&lp, Path::new("x.pdf")).unwrap_err(), "no default destination");
    }

    #[test]
    fn a_missing_print_program_says_to_save_instead() {
        let _alone = alone();
        assert!(print_with("/nonexistent/lp", Path::new("x.pdf")).unwrap_err().contains("save the PDF"));
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub fn pick_picture(done: impl FnOnce(Option<std::path::PathBuf>) + Send + 'static) {
    std::thread::spawn(move || {
        let picked = rfd::FileDialog::new()
            .set_title("Pick a picture to draw from")
            .add_filter("pictures", &["png", "jpg", "jpeg", "webp"])
            .pick_file();
        done(picked);
    });
}

pub const CAN_PICK_PICTURE: bool = cfg!(any(target_os = "linux", target_os = "macos", target_os = "windows"));

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub fn pick_picture(done: impl FnOnce(Option<std::path::PathBuf>) + Send + 'static) {
    done(None);
}
