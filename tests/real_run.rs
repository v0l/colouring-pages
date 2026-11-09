use colouring_pages::backend::{Comfy, ImageBackend, OpenRouter};
use colouring_pages::clean::{Paper, is_black_and_white};
use colouring_pages::detail::{Detail, Look};
use colouring_pages::llm::Llm;
use colouring_pages::pipeline::{Event, run};
use colouring_pages::settings::Settings;
use std::sync::{Arc, Mutex};

#[test]
#[ignore]
fn a_request_gives_four_clean_pages_on_real_services() {
    let saved = Settings::load(&colouring_pages::settings::settings_path());
    let llm = Llm {
        url: saved.llm_url.clone(),
        key: saved.llm_key.clone(),
        model: saved.llm_model.clone(),
        vision_model: std::env::var("VISION_MODEL").unwrap_or_else(|_| saved.vision_model.clone()),
    };
    assert!(!llm.key.is_empty(), "the YALR key is in settings.json");
    let backend: Arc<dyn ImageBackend> = match std::env::var("BACKEND").as_deref() {
        Ok("openrouter") => Arc::new(OpenRouter::new(&saved.openrouter_url, &saved.openrouter_key, &saved.openrouter_model)),
        _ => Arc::new(Comfy::new(&std::env::var("COMFY_URL").unwrap_or_else(|_| "http://127.0.0.1:18188".into()))),
    };
    let out = std::path::PathBuf::from(std::env::var("OUT").unwrap_or_else(|_| "shots/real".into()));
    std::fs::create_dir_all(&out).unwrap();
    let request = std::env::var("REQUEST").unwrap_or_else(|_| "cute christmas cats".into());
    let detail = Detail::from_index(std::env::var("DETAIL").ok().and_then(|d| d.parse().ok()).unwrap_or(2));
    let close = std::env::var("CLOSE").is_ok();
    let reference = std::env::var("REFERENCE").ok().map(|p| {
        let bytes = std::fs::read(p).unwrap();
        Arc::new(colouring_pages::app::reference_png(&bytes).unwrap())
    });
    let log: Arc<Mutex<Vec<Event>>> = Arc::default();
    let l = log.clone();
    let sink = Arc::new(move |e: Event| {
        eprintln!("{}", match &e {
            Event::Page(i, _) => format!("page {i}"),
            other => format!("{other:?}").chars().take(600).collect(),
        });
        l.lock().unwrap().push(e)
    });
    colouring_pages::net::runtime().block_on(run(llm, backend, Paper::A4, request, reference, Look { detail, close }, sink));
    let events = log.lock().unwrap();
    let mut n = 0;
    for e in events.iter() {
        if let Event::Page(i, img) = e {
            assert!(is_black_and_white(img));
            assert_eq!(img.dimensions(), Paper::A4.pixels());
            img.save(out.join(format!("page-{}.png", i + 1))).unwrap();
            n += 1;
        }
    }
    assert_eq!(n, 4, "{events:?}");
}
