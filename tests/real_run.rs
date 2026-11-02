use colouring_pages::backend::Comfy;
use colouring_pages::clean::{Paper, is_black_and_white};
use colouring_pages::llm::Llm;
use colouring_pages::pipeline::{Event, run};
use colouring_pages::settings::Settings;
use std::sync::{Arc, Mutex};

#[test]
#[ignore]
fn cute_christmas_cats_gives_four_clean_pages_on_real_services() {
    let saved = Settings::load(&colouring_pages::settings::settings_path());
    let llm = Llm { url: saved.llm_url.clone(), key: saved.llm_key.clone(), model: saved.llm_model.clone() };
    assert!(!llm.key.is_empty(), "the YALR key is in settings.json");
    let comfy = std::env::var("COMFY_URL").unwrap_or_else(|_| "http://127.0.0.1:18188".into());
    let out = std::path::PathBuf::from(std::env::var("OUT").unwrap_or_else(|_| "shots/real".into()));
    std::fs::create_dir_all(&out).unwrap();
    let log: Arc<Mutex<Vec<Event>>> = Arc::default();
    let l = log.clone();
    let sink = Arc::new(move |e: Event| {
        eprintln!("{}", match &e {
            Event::Page(i, _) => format!("page {i}"),
            other => format!("{other:?}").chars().take(300).collect(),
        });
        l.lock().unwrap().push(e)
    });
    colouring_pages::net::runtime().block_on(run(llm, Arc::new(Comfy::new(&comfy)), Paper::A4, "cute christmas cats".into(), sink));
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
