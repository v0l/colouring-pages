use crate::backend::{ImageBackend, Job, generation_size};
use crate::clean::{Paper, make_page};
use crate::detail::Look;
use crate::llm::Llm;
use crate::net;
use crate::plan::{Plan, PlanError, make_plan, make_prompts};
use crate::style;
use image::GrayImage;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Event {
    Planning,
    Planned(Plan),
    Drawing(Vec<String>),
    Page(usize, GrayImage),
    PageFailed(usize, String),
    Refused(String),
    Failed(String),
    Finished,
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

fn aspect(paper: Paper) -> f32 {
    let (w, h) = paper.inches();
    (w - 0.8) / (h - 0.8)
}

pub type Reference = Option<Arc<Vec<u8>>>;

pub async fn draw_page(
    backend: &dyn ImageBackend,
    prompt: &str,
    paper: Paper,
    seed: u64,
    reference: Reference,
    look: Look,
) -> Result<GrayImage, String> {
    let (width, height) = generation_size(aspect(paper));
    let job = Job { prompt: prompt.to_string(), negative: style::negative(look.detail), width, height, seed, reference, look };
    let png = backend.generate(&job).await?;
    make_page(&png, paper, look.detail)
}

const EMPTY_PAGE: &str = "the page came out empty";
const TRIES: u64 = 3;

async fn draw_page_retrying(
    backend: &dyn ImageBackend,
    prompt: &str,
    paper: Paper,
    seed: u64,
    reference: Reference,
    look: Look,
) -> Result<GrayImage, String> {
    let mut last = String::new();
    for attempt in 0..TRIES {
        match draw_page(backend, prompt, paper, seed + attempt * 7919, reference.clone(), look).await {
            Err(e) if e == EMPTY_PAGE => last = e,
            other => return other,
        }
    }
    Err(last)
}

pub async fn run(llm: Llm, backend: Arc<dyn ImageBackend>, paper: Paper, request: String, reference: Reference, look: Look, sink: Sink) {
    sink(Event::Planning);
    let plan = match make_plan(&llm, &request, look.detail, reference.as_ref().map(|r| r.as_slice())).await {
        Ok(p) => p,
        Err(PlanError::Refused(why)) => return sink(Event::Refused(why)),
        Err(e) => return sink(Event::Failed(format!("could not make a plan: {e}"))),
    };
    sink(Event::Planned(plan.clone()));
    let prompts = make_prompts(&llm, &plan).await;
    sink(Event::Drawing(prompts.clone()));
    draw_pages(backend, paper, prompts.into_iter().enumerate().collect(), reference, look, sink).await;
}

pub async fn draw_pages(
    backend: Arc<dyn ImageBackend>,
    paper: Paper,
    pages: Vec<(usize, String)>,
    reference: Reference,
    look: Look,
    sink: Sink,
) {
    let base = net::random_seed(net::unix_seconds() ^ pages.len() as u64);
    let mut tasks = Vec::new();
    for (i, prompt) in pages {
        let (backend, sink, reference) = (backend.clone(), sink.clone(), reference.clone());
        tasks.push(tokio::spawn(async move {
            match draw_page_retrying(&*backend, &prompt, paper, base + i as u64, reference, look).await {
                Ok(page) => sink(Event::Page(i, page)),
                Err(e) => sink(Event::PageFailed(i, e)),
            }
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
    sink(Event::Finished);
}

pub async fn redraw(backend: Arc<dyn ImageBackend>, paper: Paper, index: usize, prompt: String, reference: Reference, look: Look, sink: Sink) {
    draw_pages(backend, paper, vec![(index, prompt)], reference, look, sink).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Reply, serve};
    use crate::detail::Detail;
    use crate::style::STYLE_SUFFIX;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Mutex;

    struct Grey {
        jobs: Mutex<Vec<Job>>,
        fail_on: Option<usize>,
    }

    #[async_trait]
    impl ImageBackend for Grey {
        fn name(&self) -> &'static str {
            "grey"
        }

        async fn generate(&self, job: &Job) -> Result<Vec<u8>, String> {
            let n = {
                let mut jobs = self.jobs.lock().unwrap();
                jobs.push(job.clone());
                jobs.len() - 1
            };
            if self.fail_on == Some(n) {
                return Err("backend broke".into());
            }
            let img = GrayImage::from_fn(job.width / 4, job.height / 4, |x, y| {
                let d = (((x as f32 - 100.0).powi(2) + (y as f32 - 140.0).powi(2)).sqrt() - 60.0).abs();
                image::Luma([if d < 3.0 { 40 } else { 230 }])
            });
            crate::clean::encode_png(&img)
        }
    }

    fn llm() -> (crate::fake::Fake, Llm) {
        let n = std::sync::atomic::AtomicUsize::new(0);
        let fake = serve(move |_| {
            let plan = r#"{"safe":true,"subject":"cats","setting":"home","mood":"happy","picture":"a cat","ideas":["a cat on a sofa","a cat with a ball","a cat in a box","a cat by a window"]}"#;
            let prompts = r#"{"prompts":["cat sofa","cat ball","cat box","cat window"]}"#;
            let i = n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let a = if i == 0 { plan } else { prompts };
            Reply::json(json!({"choices":[{"message":{"content":a}}]}))
        });
        let llm = Llm { url: fake.url.clone(), key: String::new(), model: "m".into(), vision_model: String::new() };
        (fake, llm)
    }

    fn collect() -> (Sink, Arc<Mutex<Vec<Event>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        (Arc::new(move |e| l.lock().unwrap().push(e)), log)
    }

    #[test]
    fn a_request_becomes_four_clean_a4_pages() {
        let (_fake, llm) = llm();
        let backend = Arc::new(Grey { jobs: Mutex::new(Vec::new()), fail_on: None });
        let (sink, log) = collect();
        net::runtime().block_on(run(llm, backend.clone(), Paper::A4, "cute christmas cats".into(), None, Look::default(), sink));
        let events = log.lock().unwrap().clone();
        assert!(matches!(events.first(), Some(Event::Planning)));
        assert!(matches!(events.last(), Some(Event::Finished)));
        let pages: Vec<_> = events.iter().filter_map(|e| if let Event::Page(i, p) = e { Some((*i, p)) } else { None }).collect();
        assert_eq!(pages.len(), 4);
        for (_, p) in &pages {
            assert_eq!(p.dimensions(), Paper::A4.pixels());
            assert!(crate::clean::is_black_and_white(p));
            assert!(crate::clean::ink_ratio(p) > 0.0005);
        }
        let jobs = backend.jobs.lock().unwrap();
        assert_eq!(jobs.len(), 4);
        assert!(jobs.iter().all(|j| j.prompt.ends_with(STYLE_SUFFIX) && !j.negative.is_empty()));
        assert!(jobs.iter().all(|j| j.look == Look::default() && j.negative == style::negative(Detail::Medium)));
        let mut seeds: Vec<_> = jobs.iter().map(|j| j.seed).collect();
        seeds.dedup();
        assert_eq!(seeds.len(), 4);
    }

    #[test]
    fn one_failed_page_does_not_stop_the_others() {
        let (_fake, llm) = llm();
        let backend = Arc::new(Grey { jobs: Mutex::new(Vec::new()), fail_on: Some(1) });
        let (sink, log) = collect();
        net::runtime().block_on(run(llm, backend, Paper::Letter, "cats".into(), None, Look::default(), sink));
        let events = log.lock().unwrap().clone();
        assert_eq!(events.iter().filter(|e| matches!(e, Event::Page(..))).count(), 3);
        assert_eq!(events.iter().filter(|e| matches!(e, Event::PageFailed(_, m) if m.contains("broke"))).count(), 1);
        assert!(matches!(events.last(), Some(Event::Finished)));
    }

    #[test]
    fn a_refused_request_stops_before_any_drawing() {
        let (_fake, llm) = llm();
        let backend = Arc::new(Grey { jobs: Mutex::new(Vec::new()), fail_on: None });
        let (sink, log) = collect();
        net::runtime().block_on(run(llm, backend.clone(), Paper::A4, "a gun".into(), None, Look::default(), sink));
        let events = log.lock().unwrap().clone();
        assert!(matches!(events.last(), Some(Event::Refused(_))));
        assert!(backend.jobs.lock().unwrap().is_empty());
    }

    #[test]
    fn a_dead_language_model_fails_cleanly() {
        let llm = Llm { url: "http://127.0.0.1:1".into(), key: String::new(), model: "m".into(), vision_model: String::new() };
        let backend = Arc::new(Grey { jobs: Mutex::new(Vec::new()), fail_on: None });
        let (sink, log) = collect();
        net::runtime().block_on(run(llm, backend, Paper::A4, "cats".into(), None, Look::default(), sink));
        assert!(matches!(log.lock().unwrap().last(), Some(Event::Failed(_))));
    }

    struct BlankFirst {
        jobs: Mutex<Vec<Job>>,
    }

    #[async_trait]
    impl ImageBackend for BlankFirst {
        fn name(&self) -> &'static str {
            "blank-first"
        }

        async fn generate(&self, job: &Job) -> Result<Vec<u8>, String> {
            let n = {
                let mut jobs = self.jobs.lock().unwrap();
                jobs.push(job.clone());
                jobs.len()
            };
            let img = GrayImage::from_fn(job.width / 4, job.height / 4, |x, y| {
                let d = (((x as f32 - 100.0).powi(2) + (y as f32 - 140.0).powi(2)).sqrt() - 60.0).abs();
                image::Luma([if n > 1 && d < 3.0 { 40 } else { 230 }])
            });
            crate::clean::encode_png(&img)
        }
    }

    #[test]
    fn an_empty_page_is_drawn_again_with_a_new_seed() {
        let backend = BlankFirst { jobs: Mutex::new(Vec::new()) };
        let page = net::runtime()
            .block_on(draw_page_retrying(&backend, &style::finish("a cat", Detail::Medium), Paper::A4, 10, None, Look::default()))
            .unwrap();
        assert!(crate::clean::ink_ratio(&page) > 0.001);
        let jobs = backend.jobs.lock().unwrap();
        assert_eq!(jobs.len(), 2);
        assert_ne!(jobs[0].seed, jobs[1].seed);
    }

    #[test]
    fn redrawing_one_page_uses_the_same_prompt_with_a_new_seed() {
        let backend = Arc::new(Grey { jobs: Mutex::new(Vec::new()), fail_on: None });
        let (sink, log) = collect();
        let prompt = style::finish("cat sofa", Detail::Medium);
        net::runtime().block_on(redraw(backend.clone(), Paper::A4, 2, prompt.clone(), None, Look::default(), sink));
        let events = log.lock().unwrap().clone();
        assert!(matches!(&events[0], Event::Page(2, _)));
        assert_eq!(backend.jobs.lock().unwrap()[0].prompt, prompt);
    }

    #[test]
    fn the_level_reaches_the_prompts_negative_and_cleanup_of_every_page() {
        let (_fake, llm) = llm();
        let backend = Arc::new(Grey { jobs: Mutex::new(Vec::new()), fail_on: None });
        let (sink, _log) = collect();
        let look = Look { detail: Detail::VeryDetailed, close: true };
        net::runtime().block_on(run(llm, backend.clone(), Paper::A4, "cats".into(), Some(Arc::new(vec![1])), look, sink));
        let jobs = backend.jobs.lock().unwrap();
        assert_eq!(jobs.len(), 4);
        assert!(jobs.iter().all(|j| j.look == look && j.prompt.contains(Detail::VeryDetailed.words())));
        assert!(jobs.iter().all(|j| !j.negative.contains("tiny details")));
    }

    #[test]
    fn a_redraw_keeps_the_reference_and_the_look() {
        let backend = Arc::new(Grey { jobs: Mutex::new(Vec::new()), fail_on: None });
        let (sink, _log) = collect();
        let look = Look { detail: Detail::Simple, close: false };
        let reference: Reference = Some(Arc::new(vec![7]));
        net::runtime().block_on(redraw(backend.clone(), Paper::A4, 0, style::finish("cat", Detail::Simple), reference.clone(), look, sink));
        let jobs = backend.jobs.lock().unwrap();
        assert_eq!(jobs[0].look, look);
        assert_eq!(jobs[0].reference, reference);
        assert!(jobs[0].negative.contains("tiny details"));
    }
}
