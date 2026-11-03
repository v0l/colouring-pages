use crate::backend::ImageBackend;
use crate::clean::{self, Paper};
use crate::llm::Llm;
use crate::net;
use crate::pipeline::{self, Event, Sink};
use crate::plan::{PAGES, Plan};
use crate::store::{self, Entry, Meta, Status, Store};
use image::GrayImage;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub enum Page {
    Pending,
    Ready(Arc<GrayImage>),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Thinking,
    Drawing,
    Done,
    Failed(String),
    Refused(String),
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: String,
    pub request: String,
    pub made: u64,
    pub paper: Paper,
    pub plan: Option<Plan>,
    pub prompts: Vec<String>,
    pub pages: Vec<Page>,
    pub step: Step,
    pub status: Status,
    pub version: u64,
}

impl Job {
    pub fn running(&self) -> bool {
        self.status == Status::Running
    }

    pub fn ready(&self) -> usize {
        self.pages.iter().filter(|p| matches!(p, Page::Ready(_))).count()
    }

    fn meta(&self) -> Meta {
        Meta {
            id: self.id.clone(),
            request: self.request.clone(),
            made: self.made,
            paper: self.paper.id().into(),
            plan: self.plan.clone(),
            prompts: self.prompts.clone(),
            pages: PAGES,
            status: self.status.clone(),
        }
    }

    fn from_entry(entry: &Entry) -> Job {
        let pages = (0..PAGES)
            .map(|i| match image::open(entry.page_path(i)) {
                Ok(img) => Page::Ready(Arc::new(img.to_luma8())),
                Err(_) if entry.meta.status == Status::Running => Page::Pending,
                Err(_) => Page::Failed("this page was not drawn".into()),
            })
            .collect();
        let step = match &entry.meta.status {
            Status::Failed(why) => Step::Failed(why.clone()),
            Status::Interrupted => Step::Failed(INTERRUPTED.into()),
            _ if entry.meta.prompts.is_empty() => Step::Thinking,
            _ => Step::Done,
        };
        Job {
            id: entry.meta.id.clone(),
            request: entry.meta.request.clone(),
            made: entry.meta.made,
            paper: entry.paper(),
            plan: entry.meta.plan.clone(),
            prompts: entry.meta.prompts.clone(),
            pages,
            step,
            status: entry.meta.status.clone(),
            version: 0,
        }
    }
}

pub const INTERRUPTED: &str = "the app closed before these pages were finished";
pub const NOTHING_DRAWN: &str = "no page could be drawn, check the picture maker in settings";

type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub struct Jobs {
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    store: Store,
    wake: Wake,
    version: Arc<AtomicU64>,
}

impl Jobs {
    pub fn new(store: Store, wake: impl Fn() + Send + Sync + 'static) -> Jobs {
        store.mark_interrupted();
        Jobs { jobs: Arc::default(), store, wake: Arc::new(wake), version: Arc::default() }
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    pub fn get(&self, id: &str) -> Option<Job> {
        self.lock().get(id).cloned()
    }

    pub fn any_running(&self) -> bool {
        self.lock().values().any(Job::running)
    }

    pub fn running(&self, id: &str) -> bool {
        self.lock().get(id).is_some_and(Job::running)
    }

    pub fn open(&self, entry: &Entry) -> Job {
        if let Some(job) = self.get(&entry.meta.id) {
            return job;
        }
        let job = Job::from_entry(entry);
        self.lock().insert(job.id.clone(), job.clone());
        job
    }

    pub fn forget_idle(&self, keep: &str) {
        self.lock().retain(|id, job| job.running() || id == keep);
    }

    pub fn start(&self, llm: Llm, backend: Arc<dyn ImageBackend>, paper: Paper, request: String) -> Result<String, String> {
        let made = net::unix_seconds();
        let job = Job {
            id: store::new_id(made),
            request: request.trim().to_string(),
            made,
            paper,
            plan: None,
            prompts: Vec::new(),
            pages: (0..PAGES).map(|_| Page::Pending).collect(),
            step: Step::Thinking,
            status: Status::Running,
            version: 0,
        };
        self.store.save_meta(&job.meta())?;
        let id = job.id.clone();
        self.lock().insert(id.clone(), job);
        self.bump();
        net::spawn(pipeline::run(llm, backend, paper, request, self.sink(&id)));
        Ok(id)
    }

    pub fn redraw(&self, id: &str, index: usize, backend: Arc<dyn ImageBackend>) -> bool {
        let Some(prompt) = self.get(id).and_then(|j| j.prompts.get(index).cloned()) else { return false };
        self.draw(id, vec![(index, prompt)], backend)
    }

    pub fn retry(&self, id: &str, llm: Llm, backend: Arc<dyn ImageBackend>) -> bool {
        if self.running(id) {
            return false;
        }
        let Some(entry) = self.store.get(id) else { return false };
        let job = self.open(&entry);
        if job.prompts.is_empty() {
            self.update(id, |j| {
                j.status = Status::Running;
                j.step = Step::Thinking;
                j.pages = (0..PAGES).map(|_| Page::Pending).collect();
            });
            net::spawn(pipeline::run(llm, backend, job.paper, job.request.clone(), self.sink(id)));
            return true;
        }
        let missing: Vec<(usize, String)> = job
            .prompts
            .iter()
            .cloned()
            .enumerate()
            .filter(|(i, _)| !matches!(job.pages.get(*i), Some(Page::Ready(_))))
            .collect();
        !missing.is_empty() && self.draw(id, missing, backend)
    }

    fn draw(&self, id: &str, pages: Vec<(usize, String)>, backend: Arc<dyn ImageBackend>) -> bool {
        let Some(paper) = self.get(id).filter(|j| !j.running()).map(|j| j.paper) else { return false };
        self.update(id, |j| {
            j.status = Status::Running;
            j.step = Step::Drawing;
            for (i, _) in &pages {
                if let Some(p) = j.pages.get_mut(*i) {
                    *p = Page::Pending;
                }
            }
        });
        net::spawn(pipeline::draw_pages(backend, paper, pages, self.sink(id)));
        true
    }

    fn sink(&self, id: &str) -> Sink {
        let (jobs, id) = (self.clone(), id.to_string());
        Arc::new(move |event| jobs.handle(&id, event))
    }

    fn handle(&self, id: &str, event: Event) {
        match event {
            Event::Planning => self.update(id, |j| j.step = Step::Thinking),
            Event::Planned(plan) => self.update(id, |j| j.plan = Some(plan)),
            Event::Drawing(prompts) => self.update(id, |j| {
                j.prompts = prompts;
                j.step = Step::Drawing;
            }),
            Event::Page(i, img) => {
                if let Ok(png) = clean::encode_png(&img) {
                    let _ = self.store.save_page(id, i, &png);
                }
                self.update(id, |j| {
                    if let Some(p) = j.pages.get_mut(i) {
                        *p = Page::Ready(Arc::new(img));
                    }
                });
            }
            Event::PageFailed(i, why) => self.update(id, |j| {
                if let Some(p) = j.pages.get_mut(i) {
                    *p = Page::Failed(why);
                }
            }),
            Event::Refused(why) => {
                let _ = self.store.remove(id);
                self.update(id, |j| {
                    j.status = Status::Failed(why.clone());
                    j.step = Step::Refused(why);
                });
            }
            Event::Failed(why) => self.update(id, |j| {
                j.status = Status::Failed(why.clone());
                j.step = Step::Failed(why);
            }),
            Event::Finished => self.update(id, |j| {
                if j.ready() == 0 {
                    j.status = Status::Failed(NOTHING_DRAWN.into());
                    j.step = Step::Failed(NOTHING_DRAWN.into());
                } else {
                    j.status = Status::Done;
                    j.step = Step::Done;
                }
            }),
        }
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut Job)) {
        {
            let mut jobs = self.lock();
            let Some(job) = jobs.get_mut(id) else { return };
            change(job);
            job.version += 1;
            if !matches!(job.step, Step::Refused(_)) {
                let _ = self.store.save_meta(&job.meta());
            }
        }
        self.bump();
    }

    fn bump(&self) {
        self.version.fetch_add(1, Ordering::SeqCst);
        (self.wake)();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Job>> {
        self.jobs.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::backend::Job as Draw;
    use crate::fake::{Fake, Reply, serve};
    use async_trait::async_trait;
    use serde_json::json;
    use std::time::{Duration, Instant};

    pub struct Gate {
        open: Mutex<bool>,
        turn: std::sync::Condvar,
    }

    impl Gate {
        pub fn shut() -> Arc<Gate> {
            Arc::new(Gate { open: Mutex::new(false), turn: std::sync::Condvar::new() })
        }

        pub fn open(&self) {
            *self.open.lock().unwrap() = true;
            self.turn.notify_all();
        }

        pub fn wait_open(&self) {
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.turn.wait(open).unwrap();
            }
        }
    }

    pub struct Pens {
        pub gate: Arc<Gate>,
        pub broken: bool,
    }

    #[async_trait]
    impl ImageBackend for Pens {
        fn name(&self) -> &'static str {
            "pens"
        }

        async fn generate(&self, job: &Draw) -> Result<Vec<u8>, String> {
            let gate = self.gate.clone();
            tokio::task::spawn_blocking(move || gate.wait_open()).await.map_err(|e| e.to_string())?;
            if self.broken {
                return Err("the pens are dry".into());
            }
            let img = GrayImage::from_fn(job.width / 4, job.height / 4, |x, y| {
                let d = (((x as f32 - 100.0).powi(2) + (y as f32 - 140.0).powi(2)).sqrt() - 60.0).abs();
                image::Luma([if d < 3.0 { 40 } else { 230 }])
            });
            clean::encode_png(&img)
        }
    }

    pub fn thinker() -> (Fake, Llm) {
        let fake = serve(|seen| {
            let plan = r#"{"safe":true,"subject":"cats","setting":"home","mood":"happy","age":"young","ideas":["a cat on a sofa","a cat with a ball","a cat in a box","a cat by a window"]}"#;
            let prompts = r#"{"prompts":["cat sofa","cat ball","cat box","cat window"]}"#;
            let answer = if seen.text().contains("image prompts") { prompts } else { plan };
            Reply::json(json!({"choices":[{"message":{"content":answer}}]}))
        });
        let llm = Llm { url: fake.url.clone(), key: String::new(), model: "m".into() };
        (fake, llm)
    }

    pub fn until(what: &str, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn setup() -> (tempfile::TempDir, Store, Jobs) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let jobs = Jobs::new(store.clone(), || {});
        (dir, store, jobs)
    }

    #[test]
    fn a_started_request_is_saved_as_running_before_anything_is_drawn() {
        let (_dir, store, jobs) = setup();
        let (_fake, llm) = thinker();
        let gate = Gate::shut();
        let id = jobs.start(llm, Arc::new(Pens { gate: gate.clone(), broken: false }), Paper::A4, "cats".into()).unwrap();
        let entry = store.get(&id).unwrap();
        assert_eq!(entry.meta.status, Status::Running);
        assert_eq!(entry.meta.request, "cats");
        gate.open();
        until("the job to finish", || !jobs.running(&id));
    }

    #[test]
    fn pages_land_on_disk_as_they_finish_and_the_set_ends_done() {
        let (_dir, store, jobs) = setup();
        let (_fake, llm) = thinker();
        let gate = Gate::shut();
        let id = jobs.start(llm, Arc::new(Pens { gate: gate.clone(), broken: false }), Paper::A4, "cats".into()).unwrap();
        until("the prompts", || jobs.get(&id).is_some_and(|j| j.prompts.len() == PAGES));
        assert_eq!(jobs.get(&id).unwrap().ready(), 0);
        assert_eq!(store.get(&id).unwrap().meta.prompts.len(), PAGES);
        gate.open();
        until("the job to finish", || !jobs.running(&id));
        let entry = store.get(&id).unwrap();
        assert_eq!(entry.meta.status, Status::Done);
        assert!((0..PAGES).all(|i| entry.page_path(i).exists()));
        assert_eq!(jobs.get(&id).unwrap().step, Step::Done);
    }

    #[test]
    fn when_every_page_fails_the_set_is_failed_and_retry_draws_them_again() {
        let (_dir, store, jobs) = setup();
        let (_fake, llm) = thinker();
        let gate = Gate::shut();
        gate.open();
        let id = jobs.start(llm.clone(), Arc::new(Pens { gate: gate.clone(), broken: true }), Paper::A4, "cats".into()).unwrap();
        until("the job to fail", || !jobs.running(&id));
        assert_eq!(store.get(&id).unwrap().meta.status, Status::Failed(NOTHING_DRAWN.into()));
        assert!(jobs.retry(&id, llm, Arc::new(Pens { gate, broken: false })));
        until("the retry to finish", || !jobs.running(&id));
        assert_eq!(store.get(&id).unwrap().meta.status, Status::Done);
        assert_eq!(jobs.get(&id).unwrap().ready(), PAGES);
    }

    #[test]
    fn an_interrupted_set_is_finished_by_retry_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let id = {
            let jobs = Jobs::new(store.clone(), || {});
            let (_fake, llm) = thinker();
            let gate = Gate::shut();
            let id = jobs.start(llm, Arc::new(Pens { gate, broken: false }), Paper::A4, "cats".into()).unwrap();
            until("the prompts", || jobs.get(&id).is_some_and(|j| j.prompts.len() == PAGES));
            id
        };
        let jobs = Jobs::new(store.clone(), || {});
        let entry = store.get(&id).unwrap();
        assert_eq!(entry.meta.status, Status::Interrupted);
        assert_eq!(jobs.open(&entry).step, Step::Failed(INTERRUPTED.into()));
        let (_fake, llm) = thinker();
        let gate = Gate::shut();
        gate.open();
        assert!(jobs.retry(&id, llm, Arc::new(Pens { gate, broken: false })));
        until("the retry to finish", || !jobs.running(&id));
        assert_eq!(store.get(&id).unwrap().meta.status, Status::Done);
    }

    #[test]
    fn a_refused_request_leaves_nothing_in_my_pages() {
        let (_dir, store, jobs) = setup();
        let (_fake, llm) = thinker();
        let id = jobs.start(llm, Arc::new(Pens { gate: Gate::shut(), broken: false }), Paper::A4, "a gun".into()).unwrap();
        until("the refusal", || !jobs.running(&id));
        assert!(matches!(jobs.get(&id).unwrap().step, Step::Refused(_)));
        assert!(store.get(&id).is_none());
    }
}
