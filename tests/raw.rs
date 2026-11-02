use colouring_pages::backend::{Comfy, ImageBackend, Job, generation_size};
use colouring_pages::clean::{Paper, clean, otsu};

#[test]
#[ignore]
fn raw_flux_output_for_one_prompt() {
    let url = std::env::var("COMFY_URL").unwrap_or_else(|_| "http://127.0.0.1:18188".into());
    let prompt = std::env::var("PROMPT").unwrap();
    let out = std::env::var("OUT").unwrap_or_else(|_| "shots/raw".into());
    std::fs::create_dir_all(&out).unwrap();
    let (width, height) = generation_size(0.707);
    let job = Job { prompt: colouring_pages::style::finish(&prompt), negative: String::new(), width, height, seed: std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(5) };
    let png = colouring_pages::net::runtime().block_on(Comfy::new(&url).generate(&job)).unwrap();
    std::fs::write(format!("{out}/raw.png"), &png).unwrap();
    let gray = image::load_from_memory(&png).unwrap().to_luma8();
    let min = gray.pixels().map(|p| p.0[0]).min().unwrap();
    eprintln!("size {:?} min {min} otsu {}", gray.dimensions(), otsu(&gray));
    let cleaned = clean(&gray);
    eprintln!("ink after clean {}", colouring_pages::clean::ink_ratio(&cleaned));
    cleaned.save(format!("{out}/clean.png")).unwrap();
    colouring_pages::clean::make_page(&png, Paper::A4).unwrap().save(format!("{out}/page.png")).unwrap();
}
