use crate::clean::Paper;
use crate::{export, platform, settings};
use image::GrayImage;
use std::path::PathBuf;

pub fn export_dir() -> PathBuf {
    #[cfg(not(target_os = "android"))]
    {
        if let Some(pictures) = dirs::picture_dir() {
            return pictures.join("colouring-pages");
        }
    }
    settings::data_dir().join("exports")
}

pub fn slug(request: &str) -> String {
    let mut out = String::new();
    for c in request.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(40).collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "colouring-pages".into() } else { out }
}

fn scratch(name: &str) -> Result<PathBuf, String> {
    let dir = settings::data_dir().join("scratch");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(name))
}

pub fn save_pdf(pages: &[&GrayImage], paper: Paper, name: &str) -> Result<String, String> {
    let bytes = export::pdf(pages, paper)?;
    let file = format!("{name}.pdf");
    let temp = scratch(&file)?;
    std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    platform::save_download(&temp.to_string_lossy(), &file, "application/pdf")?;
    Ok(saved_where(&file))
}

pub fn save_png(pages: &[&GrayImage], name: &str) -> Result<String, String> {
    if pages.is_empty() {
        return Err("there are no pages to save".into());
    }
    let mut last = String::new();
    for (i, page) in pages.iter().enumerate() {
        let file = if pages.len() == 1 { format!("{name}.png") } else { format!("{name}-{}.png", i + 1) };
        let temp = scratch(&file)?;
        std::fs::write(&temp, export::png(page)?).map_err(|e| e.to_string())?;
        platform::save_download(&temp.to_string_lossy(), &file, "image/png")?;
        last = file;
    }
    Ok(saved_where(&last))
}

pub fn print(pages: &[&GrayImage], paper: Paper, name: &str) -> Result<String, String> {
    let temp = scratch(&format!("{name}-print.pdf"))?;
    std::fs::write(&temp, export::pdf(pages, paper)?).map_err(|e| e.to_string())?;
    platform::print_pdf(&temp.to_string_lossy())
}

fn saved_where(file: &str) -> String {
    if cfg!(target_os = "android") {
        format!("saved {file} to downloads")
    } else {
        format!("saved to {}", export_dir().join(file).display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_for_files() {
        assert_eq!(slug("Cute Christmas Cats!"), "cute-christmas-cats");
        assert_eq!(slug("../../etc/passwd"), "etc-passwd");
        assert_eq!(slug("   "), "colouring-pages");
        assert_eq!(slug("!!!"), "colouring-pages");
        assert!(slug(&"a".repeat(200)).len() <= 40);
    }
}
