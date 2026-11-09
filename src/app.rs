use crate::backend::{ImageBackend, Kind};
use crate::clean::Paper;
use crate::detail::Detail;
use crate::jobs::{Job, Jobs, Page, Step};
use crate::plan::PAGES;
use crate::settings::{self, Listen, Settings};
use crate::store::{Entry, Status, Store};
use crate::voice::{self, Said, Voice};
use crate::outputs;
use egui::{Color32, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Ui, pos2, vec2};
use egui_bench::prelude::*;
use image::GrayImage;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

const HOLD_TO_OPEN: f32 = 1.5;
const BIG: f32 = 64.0;
const DETAIL_ROW_H: f32 = 62.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Home,
    Job,
    Gallery,
}

enum Msg {
    Notice(bool, String),
    Picked(Result<Vec<u8>, String>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Done,
    Pending,
    Missing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Export {
    Print,
    Pdf,
    Png,
}

struct View {
    job: Job,
    textures: Vec<Option<(usize, TextureHandle)>>,
    reference: Option<TextureHandle>,
}

pub struct App {
    settings: Settings,
    store: Store,
    jobs: Jobs,
    voice: Voice,
    screen: Screen,
    request: String,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    current: Option<String>,
    view: Option<View>,
    selected: Vec<bool>,
    notice: Option<(bool, String)>,
    notice_until: Option<Instant>,
    draft: Option<Settings>,
    hold: Option<Instant>,
    gallery: Vec<Entry>,
    gallery_seen: u64,
    thumbs: HashMap<String, (usize, TextureHandle)>,
    removing: Option<String>,
    level: f32,
    auto_hold: Option<(Option<Instant>, f32)>,
    reference: Option<(Arc<Vec<u8>>, TextureHandle)>,
}

pub fn reference_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("that picture could not be read: {e}"))?;
    let img = if img.width().max(img.height()) > 1024 { img.thumbnail(1024, 1024) } else { img };
    let mut out = std::io::Cursor::new(Vec::new());
    img.to_rgb8().write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        egui_bench::install(&cc.egui_ctx);
        let settings = Settings::load(&settings::settings_path());
        let store = Store::new(&settings::data_dir());
        let mut app = App::with(&cc.egui_ctx, settings, store);
        app.auto_hold = std::env::var("COLOURING_AUTOHOLD").ok().and_then(|v| v.parse().ok()).map(|secs| (None, secs));
        app.apply_env(&cc.egui_ctx);
        app
    }

    pub fn with(ctx: &egui::Context, settings: Settings, store: Store) -> App {
        let (tx, rx) = channel();
        let wake = ctx.clone();
        App {
            jobs: Jobs::new(store.clone(), move || wake.request_repaint()),
            store,
            settings,
            voice: Voice::default(),
            screen: Screen::Home,
            request: String::new(),
            tx,
            rx,
            current: None,
            view: None,
            selected: vec![false; PAGES],
            notice: None,
            notice_until: None,
            draft: None,
            hold: None,
            gallery: Vec::new(),
            gallery_seen: u64::MAX,
            thumbs: HashMap::new(),
            removing: None,
            level: 0.0,
            auto_hold: None,
            reference: None,
        }
    }

    fn apply_env(&mut self, ctx: &egui::Context) {
        match std::env::var("COLOURING_SHOW").as_deref() {
            Ok("settings") => self.draft = Some(self.settings.clone()),
            Ok("gallery") => self.open_gallery(ctx),
            Ok("results") => {
                if let Some(entry) = self.store.list().first().cloned() {
                    self.open_entry(ctx, &entry);
                }
            }
            _ => {}
        }
        if let Ok(path) = std::env::var("COLOURING_REFERENCE") {
            match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| reference_png(&b)) {
                Ok(png) => self.use_reference(ctx, png),
                Err(e) => self.say(false, e),
            }
        }
        if let Ok(text) = std::env::var("COLOURING_AUTORUN") {
            if !text.trim().is_empty() {
                self.request = text.clone();
                self.start(ctx, text);
            }
        }
        if std::env::var("COLOURING_AUTORUN_THEN").as_deref() == Ok("gallery") {
            self.open_gallery(ctx);
        }
    }

    fn voice_settings(&self) -> voice::Settings {
        voice::Settings {
            here: self.settings.listen == Listen::Here,
            model: self.settings.whisper_model.clone(),
            url: self.settings.speech_url.clone(),
            key: self.settings.speech_key.clone(),
            server_model: self.settings.speech_model.clone(),
        }
    }

    fn say(&mut self, ok: bool, text: impl Into<String>) {
        self.notice = Some((ok, text.into()));
        self.notice_until = Some(Instant::now() + Duration::from_secs(if ok { 6 } else { 10 }));
    }

    fn backend(&self) -> Arc<dyn ImageBackend> {
        Arc::from(self.settings.image_backend())
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn current(&self) -> Option<&Job> {
        self.view.as_ref().map(|v| &v.job)
    }

    pub fn gallery(&self) -> &[Entry] {
        &self.gallery
    }

    pub fn start(&mut self, ctx: &egui::Context, request: String) {
        let request = request.trim().to_string();
        if request.is_empty() {
            return;
        }
        if let Some(problem) = self.settings.problem() {
            self.say(false, problem);
            return;
        }
        let reference = self.reference.take().map(|(png, _)| png);
        match self.jobs.start(self.settings.llm(), self.backend(), self.settings.paper(), request, reference, self.settings.look()) {
            Ok(id) => self.show_job(ctx, id),
            Err(e) => self.say(false, format!("could not keep these pages: {e}")),
        }
    }

    pub fn use_reference(&mut self, ctx: &egui::Context, png: Vec<u8>) {
        let Ok(img) = image::load_from_memory(&png) else { return };
        let tex = Self::texture(ctx, "reference", &img.to_luma8(), 160);
        self.reference = Some((Arc::new(png), tex));
    }

    fn pick_reference(&self, ctx: &egui::Context) {
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        crate::platform::pick_picture(move |picked| {
            let Some(path) = picked else { return };
            let read = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| reference_png(&b));
            let _ = tx.send(Msg::Picked(read));
            ctx.request_repaint();
        });
    }

    fn show_job(&mut self, ctx: &egui::Context, id: String) {
        if self.current.as_deref() != Some(id.as_str()) {
            self.selected = vec![false; PAGES];
            self.view = None;
        }
        self.jobs.forget_idle(&id);
        self.current = Some(id);
        self.screen = Screen::Job;
        self.sync(ctx);
    }

    pub fn open_entry(&mut self, ctx: &egui::Context, entry: &Entry) {
        self.jobs.open(entry);
        self.request = entry.meta.request.clone();
        self.show_job(ctx, entry.meta.id.clone());
    }

    pub fn go_home(&mut self) {
        self.screen = Screen::Home;
        self.request.clear();
    }

    pub fn back_from_gallery(&mut self) {
        self.screen = if self.view.is_some() { Screen::Job } else { Screen::Home };
    }

    fn redraw(&mut self, ctx: &egui::Context, index: usize) {
        let Some(id) = self.current.clone() else { return };
        if self.jobs.redraw(&id, index, self.backend()) {
            self.selected[index] = false;
            self.sync(ctx);
        }
    }

    pub fn retry(&mut self, ctx: &egui::Context, id: &str) {
        if let Some(problem) = self.settings.problem() {
            self.say(false, problem);
            return;
        }
        if self.jobs.retry(id, self.settings.llm(), self.backend()) {
            if self.current.as_deref() == Some(id) {
                self.sync(ctx);
            }
            if self.screen == Screen::Gallery {
                self.refresh_gallery(ctx);
            }
        }
    }

    fn texture(ctx: &egui::Context, name: &str, img: &GrayImage, width: u32) -> TextureHandle {
        let small = image::imageops::resize(
            img,
            width,
            (img.height() as f32 * width as f32 / img.width() as f32).round().max(1.0) as u32,
            image::imageops::FilterType::Triangle,
        );
        let color = egui::ColorImage::from_gray([small.width() as usize, small.height() as usize], small.as_raw());
        ctx.load_texture(name, color, TextureOptions::LINEAR)
    }

    pub fn sync(&mut self, ctx: &egui::Context) {
        let Some(id) = self.current.clone() else { return };
        let Some(job) = self.jobs.get(&id) else { return };
        if let Step::Refused(why) = &job.step {
            let why = why.clone();
            self.current = None;
            self.view = None;
            if self.screen == Screen::Job {
                self.screen = Screen::Home;
            }
            self.say(false, why);
            return;
        }
        if self.view.as_ref().is_some_and(|v| v.job.version == job.version && v.job.id == job.id) {
            return;
        }
        let (mut textures, kept) = match self.view.take() {
            Some(v) if v.job.id == job.id => (v.textures, v.reference),
            _ => (vec![None; PAGES], None),
        };
        for (i, page) in job.pages.iter().enumerate() {
            textures[i] = match page {
                Page::Ready(img) => {
                    let key = Arc::as_ptr(img) as usize;
                    match textures[i].take() {
                        Some((k, tex)) if k == key => Some((k, tex)),
                        _ => Some((key, Self::texture(ctx, &format!("page-{}-{i}-{key}", job.id), img, 700))),
                    }
                }
                _ => None,
            };
            if !matches!(page, Page::Ready(_)) {
                self.selected[i] = false;
            }
        }
        let reference = match kept {
            Some(tex) => Some(tex),
            None => job.reference.as_ref().and_then(|png| image::load_from_memory(png).ok()).map(|img| {
                Self::texture(ctx, &format!("job-reference-{}", job.id), &img.to_luma8(), 160)
            }),
        };
        self.view = Some(View { job, textures, reference });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Notice(ok, text) => self.say(ok, text),
                Msg::Picked(Ok(png)) => self.use_reference(ctx, png),
                Msg::Picked(Err(e)) => self.say(false, e),
            }
        }
        for said in self.voice.poll() {
            match said {
                Said::Words(t) if t.trim().is_empty() => self.say(false, "I did not catch that, try again"),
                Said::Words(t) => {
                    self.request = t.clone();
                    self.start(ctx, t);
                }
                Said::Failed(e) => self.say(false, e),
            }
        }
        self.sync(ctx);
        if self.screen == Screen::Gallery && self.gallery_seen != self.jobs.version() {
            self.refresh_gallery(ctx);
        }
        if let Some(until) = self.notice_until {
            if Instant::now() > until {
                self.notice = None;
                self.notice_until = None;
            }
        }
    }

    pub fn open_gallery(&mut self, ctx: &egui::Context) {
        self.refresh_gallery(ctx);
        self.screen = Screen::Gallery;
    }

    fn refresh_gallery(&mut self, ctx: &egui::Context) {
        self.gallery_seen = self.jobs.version();
        self.gallery = self.store.list();
        let ids: Vec<String> = self.gallery.iter().map(|e| e.meta.id.clone()).collect();
        self.thumbs.retain(|id, _| ids.contains(id));
        for entry in &self.gallery {
            let found: Vec<usize> = (0..PAGES).filter(|i| entry.page_path(*i).exists()).collect();
            if self.thumbs.get(&entry.meta.id).is_some_and(|(n, _)| *n == found.len()) {
                continue;
            }
            let Some(first) = found.first() else {
                self.thumbs.remove(&entry.meta.id);
                continue;
            };
            if let Ok(img) = image::open(entry.page_path(*first)) {
                let tex = Self::texture(ctx, &format!("thumb-{}-{}", entry.meta.id, found.len()), &img.to_luma8(), 360);
                self.thumbs.insert(entry.meta.id.clone(), (found.len(), tex));
            }
        }
    }

    fn page_marks(&self, entry: &Entry) -> Vec<Mark> {
        if let Some(job) = self.jobs.get(&entry.meta.id) {
            return job
                .pages
                .iter()
                .map(|p| match p {
                    Page::Ready(_) => Mark::Done,
                    Page::Pending if job.running() => Mark::Pending,
                    _ => Mark::Missing,
                })
                .collect();
        }
        (0..PAGES).map(|i| if entry.page_path(i).exists() { Mark::Done } else { Mark::Missing }).collect()
    }

    fn export(&mut self, ctx: &egui::Context, kind: Export) {
        let Some(view) = &self.view else { return };
        let pages: Vec<GrayImage> = view
            .job
            .pages
            .iter()
            .zip(&self.selected)
            .filter_map(|(p, on)| match p {
                Page::Ready(img) if *on => Some((**img).clone()),
                _ => None,
            })
            .collect();
        if pages.is_empty() {
            self.say(false, "tap the pages you want first");
            return;
        }
        let (paper, name, tx, ctx) = (view.job.paper, outputs::slug(&view.job.request), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let refs: Vec<&GrayImage> = pages.iter().collect();
            let done = match kind {
                Export::Print => outputs::print(&refs, paper, &name).map(|printer| format!("sent to {printer}")),
                Export::Pdf => outputs::save_pdf(&refs, paper, &name),
                Export::Png => outputs::save_png(&refs, &name),
            };
            let _ = tx.send(match done {
                Ok(text) => Msg::Notice(true, text),
                Err(e) => Msg::Notice(false, e),
            });
            ctx.request_repaint();
        });
    }
}

fn hot(resp: &egui::Response, enabled: bool) -> bool {
    enabled && (resp.hovered() || resp.has_focus())
}

fn big_button(ui: &mut Ui, rect: Rect, label: &str, tint: Color32, solid: bool, enabled: bool) -> egui::Response {
    sized_button(ui, rect, label, tint, solid, enabled, 19.0)
}

fn sized_button(ui: &mut Ui, rect: Rect, label: &str, tint: Color32, solid: bool, enabled: bool, size: f32) -> egui::Response {
    let id = egui::Id::unique(("big", label, rect.min.x as i32, rect.min.y as i32));
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    let down = enabled && resp.is_pointer_button_down_on();
    let (fill, edge, ink) = match (enabled, solid) {
        (false, _) => (WELL, ETCH, LEGEND.gamma_multiply(0.55)),
        (true, true) => {
            let f = if down { tint.gamma_multiply(0.8) } else if hot(&resp, true) { tint.lerp_to_gamma(Color32::WHITE, 0.2) } else { tint };
            (f, f, WELL)
        }
        (true, false) => (if hot(&resp, true) { BAND } else { WELL }, if hot(&resp, true) { tint } else { ETCH }, tint),
    };
    let p = ui.painter();
    p.rect_filled(rect, 2.0, fill);
    p.rect_stroke(rect, 2.0, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
    let job = egui_bench::text::caps(label, legend_font(size), ink, 2.0);
    let galley = egui_bench::text::lay(ui, job, rect.width() - 12.0);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ink);
    if hot(&resp, enabled) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn paint_spinner(p: &egui::Painter, c: Pos2, r: f32, t: f32, ink: Color32) {
    let start = t * 4.0;
    let arc: Vec<Pos2> = (0..=32)
        .map(|i| {
            let a = start + std::f32::consts::PI * 0.6 * i as f32 / 32.0;
            pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    p.add(egui::Shape::line(arc, Stroke::new(5.0, ink)));
}

fn paint_mic(p: &egui::Painter, c: Pos2, r: f32, ink: Color32) {
    let body = Rect::from_center_size(c + vec2(0.0, -r * 0.12), vec2(r * 0.42, r * 0.8));
    p.rect_filled(body, r * 0.21, ink);
    let stroke = Stroke::new(r * 0.07, ink);
    let cx = c.x;
    let top = c.y + r * 0.02;
    let rad = r * 0.4;
    let mut arc = Vec::new();
    for i in 0..=24 {
        let a = std::f32::consts::PI * i as f32 / 24.0;
        arc.push(pos2(cx - rad * a.cos(), top + rad * a.sin()));
    }
    p.add(egui::Shape::line(arc, stroke));
    p.line_segment([pos2(cx, top + rad), pos2(cx, c.y + r * 0.72)], stroke);
    p.line_segment([pos2(cx - r * 0.22, c.y + r * 0.72), pos2(cx + r * 0.22, c.y + r * 0.72)], stroke);
}


impl App {
    fn header(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let narrow = ui.available_width() < 600.0;
        let (title, spacing, key_w) = if narrow { (12.0, 1.0, 104.0) } else { (18.0, 3.0, 130.0) };
        ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new("COLOURING PAGES").font(legend_font(title)).color(VALUE).extra_letter_spacing(spacing));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(key_w, 40.0), Sense::hover());
                self.parent_button(ui, rect, if narrow { 12.0 } else { 19.0 });
                let (rect, _) = ui.allocate_exact_size(vec2(key_w, 40.0), Sense::hover());
                let label = if self.screen == Screen::Gallery { "back" } else { "my pages" };
                let key_font = if narrow { 12.0 } else { 19.0 };
                if sized_button(ui, rect, label, VALUE, false, true, key_font).clicked() {
                    if self.screen == Screen::Gallery {
                        self.back_from_gallery();
                    } else {
                        self.open_gallery(&ctx);
                    }
                }
                if !narrow && self.jobs.any_running() && self.screen != Screen::Job {
                    let (rect, _) = ui.allocate_exact_size(vec2(150.0, 40.0), Sense::hover());
                    busy(ui, rect, None, "drawing", TRACE);
                }
            });
        });
    }

    fn whisper_loader(&mut self, ui: &mut Ui, rect: Rect) {
        let status = if self.voice.status.is_empty() { "listening".to_string() } else { self.voice.status.clone() };
        let label = format!("{status}...");
        busy(ui, rect, None, &label, TRACE);
    }

    fn job_screen(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(job) = self.view.as_ref().map(|v| v.job.clone()) else {
            self.screen = Screen::Home;
            return;
        };
        let avail = ui.available_rect_before_wrap();
        if job.prompts.is_empty() {
            let w = avail.width().min(560.0);
            let rect = Rect::from_center_size(avail.center(), vec2(w, 90.0));
            match &job.step {
                Step::Failed(why) => {
                    let p = ui.painter();
                    p.rect_filled(rect, 2.0, WELL);
                    p.rect_stroke(rect, 2.0, Stroke::new(1.0, FAULT), egui::StrokeKind::Inside);
                    let g = egui_bench::text::lay(ui, egui_bench::text::caps(why, legend_font(14.0), FAULT, 1.0), w - 24.0);
                    ui.painter().galley(rect.center() - g.size() / 2.0, g, FAULT);
                    let key = Rect::from_center_size(rect.center() + vec2(0.0, 90.0), vec2(220.0, BIG));
                    if big_button(ui, key, "try again", READOUT, true, true).clicked() {
                        self.retry(&ctx, &job.id);
                    }
                }
                _ => {
                    busy(ui, rect, None, "thinking...", TRACE);
                }
            }
            if job.reference.is_some() {
                let side = 96.0;
                self.reference_thumb(ui, Rect::from_center_size(pos2(rect.center().x, rect.top() - 130.0), vec2(side, side)));
            }
            let cap = Rect::from_min_size(pos2(rect.left(), rect.top() - 46.0), vec2(w, 30.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(cap), |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(job.request.trim()).font(legend_font(16.0)).color(VALUE));
                });
            });
            return;
        }
        let banner_h = 44.0;
        let banner = Rect::from_min_size(avail.min, vec2(avail.width(), banner_h));
        self.banner(ui, &ctx, &job, banner);
        let body = Rect::from_min_max(pos2(avail.left(), avail.top() + banner_h + 10.0), avail.max);
        let gap = 14.0;
        let grid = Rect::from_min_max(body.min, pos2(body.max.x, body.max.y - BIG - 12.0));
        let landscape = grid.width() > grid.height() * 1.3;
        let (cols, rows) = if landscape { (4usize, 1usize) } else { (2, 2) };
        let cw = (grid.width() - gap * (cols as f32 - 1.0)) / cols as f32;
        let ch = (grid.height() - gap * (rows as f32 - 1.0)) / rows as f32;
        for i in 0..PAGES {
            let (r, c) = (i / cols, i % cols);
            let cell = Rect::from_min_size(grid.min + vec2(c as f32 * (cw + gap), r as f32 * (ch + gap)), vec2(cw, ch));
            self.tile(ui, &ctx, &job, i, cell);
        }
        let any = job.pages.iter().zip(&self.selected).any(|(p, on)| *on && matches!(p, Page::Ready(_)));
        let bar = Rect::from_min_max(pos2(body.left(), body.bottom() - BIG), body.max);
        let n = 4.0;
        let bw = (bar.width() - gap * (n - 1.0)) / n;
        let at = |k: f32| Rect::from_min_size(pos2(bar.left() + k * (bw + gap), bar.top()), vec2(bw, bar.height()));
        if big_button(ui, at(0.0), "new pages", VALUE, false, true).clicked() {
            self.go_home();
        }
        if big_button(ui, at(1.0), "print", READOUT, true, any).clicked() {
            self.export(&ctx, Export::Print);
        }
        if big_button(ui, at(2.0), "save pdf", VALUE, false, any).clicked() {
            self.export(&ctx, Export::Pdf);
        }
        if big_button(ui, at(3.0), "save png", VALUE, false, any).clicked() {
            self.export(&ctx, Export::Png);
        }
    }

    fn reference_thumb(&self, ui: &mut Ui, rect: Rect) {
        let Some(tex) = self.view.as_ref().and_then(|v| v.reference.clone()) else { return };
        ui.painter().rect_filled(rect, 2.0, Color32::WHITE);
        let size = tex.size_vec2();
        let fit = (rect.width() / size.x).min(rect.height() / size.y);
        ui.painter().image(
            tex.id(),
            Rect::from_center_size(rect.center(), size * fit),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(2.0, READOUT), egui::StrokeKind::Outside);
    }

    fn banner(&mut self, ui: &mut Ui, ctx: &egui::Context, job: &Job, rect: Rect) {
        let thumb_w = if job.reference.is_some() { rect.height() + 10.0 } else { 0.0 };
        if thumb_w > 0.0 {
            self.reference_thumb(ui, Rect::from_min_size(pos2(rect.left() + 2.0, rect.top() + 2.0), vec2(rect.height() - 4.0, rect.height() - 4.0)));
        }
        let rect = Rect::from_min_max(pos2(rect.left() + thumb_w, rect.top()), rect.max);
        let ready = job.ready();
        let missing = PAGES - ready;
        let (text, tint) = match (&job.step, job.running()) {
            (_, true) => (format!("{}  ·  drawing, {ready} of {PAGES} ready", job.request.trim()), TRACE),
            (Step::Failed(why), false) => (format!("{}  ·  {why}", job.request.trim()), FAULT),
            _ if missing > 0 => (format!("{}  ·  {missing} could not be drawn", job.request.trim()), FAULT),
            _ => (format!("{}  ·  tap the pages you want to print", job.request.trim()), LEGEND),
        };
        let retry_w = if !job.running() && missing > 0 { 220.0 } else { 0.0 };
        let text_rect = Rect::from_min_max(rect.min, pos2(rect.right() - retry_w - 12.0, rect.bottom()));
        if job.running() {
            busy(ui, text_rect, Some(ready as f32 / PAGES as f32), "", TRACE);
        }
        let g = egui_bench::text::lay(ui, egui_bench::text::caps(&text, legend_font(14.0), tint, 1.0), text_rect.width() - 20.0);
        ui.painter().galley(pos2(text_rect.left() + 10.0, text_rect.center().y - g.size().y / 2.0), g, tint);
        if retry_w > 0.0 {
            let key = Rect::from_min_max(pos2(rect.right() - retry_w, rect.top()), rect.max);
            let label = if ready == 0 { "try again" } else { "finish the rest" };
            if big_button(ui, key, label, READOUT, true, true).clicked() {
                self.retry(ctx, &job.id);
            }
        }
    }

    fn tile(&mut self, ui: &mut Ui, ctx: &egui::Context, job: &Job, i: usize, cell: Rect) {
        let key_h = 52.0;
        let area = Rect::from_min_max(cell.min, pos2(cell.max.x, cell.max.y - key_h - 8.0));
        let (pw, ph) = job.paper.pixels();
        let aspect = pw as f32 / ph as f32;
        let (mut w, mut h) = (area.width(), area.width() / aspect);
        if h > area.height() {
            h = area.height();
            w = h * aspect;
        }
        let page = Rect::from_center_size(area.center(), vec2(w.max(10.0), h.max(10.0)));
        let on = self.selected[i];
        let id = egui::Id::unique(("tile", i));
        let tex = self.view.as_ref().and_then(|v| v.textures[i].as_ref().map(|(_, t)| t.clone()));
        let resp = ui.interact(page, id, if tex.is_some() { Sense::click() } else { Sense::hover() });
        if resp.clicked() {
            self.selected[i] = !on;
        }
        match (&job.pages[i], tex) {
            (Page::Ready(_), Some(tex)) => {
                let p = ui.painter();
                p.rect_filled(page, 2.0, Color32::WHITE);
                p.image(tex.id(), page, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                let (c, wd) = if on { (READOUT, 4.0) } else { (ETCH, 1.0) };
                p.rect_stroke(page.expand(wd / 2.0), 2.0, Stroke::new(wd, c), egui::StrokeKind::Outside);
                if on {
                    let badge = Rect::from_center_size(page.right_top() + vec2(-20.0, 20.0), vec2(30.0, 30.0));
                    p.circle_filled(badge.center(), 15.0, READOUT);
                    let tick = [badge.center() + vec2(-6.0, 0.0), badge.center() + vec2(-2.0, 5.0), badge.center() + vec2(7.0, -5.0)];
                    p.add(egui::Shape::line(tick.to_vec(), Stroke::new(3.0, WELL)));
                }
            }
            (Page::Pending, _) if job.running() => {
                busy(ui, page, None, "drawing", TRACE);
            }
            (page_state, _) => {
                let why = match page_state {
                    Page::Failed(why) => why.clone(),
                    _ => "not drawn yet".to_string(),
                };
                let p = ui.painter();
                p.rect_filled(page, 2.0, WELL);
                p.rect_stroke(page, 2.0, Stroke::new(1.0, FAULT), egui::StrokeKind::Inside);
                let job_text = egui_bench::text::caps("could not draw this one", legend_font(13.0), FAULT, 1.0);
                let g = egui_bench::text::lay(ui, job_text, page.width() - 24.0);
                ui.painter().galley(page.center() - g.size() / 2.0 - vec2(0.0, 40.0), g, FAULT);
                let reason = egui_bench::text::lay(ui, egui_bench::text::caps(&why, legend_font(11.0), LEGEND, 0.5), page.width() - 24.0);
                ui.painter().galley(pos2(page.left() + 12.0, page.center().y), reason, LEGEND);
            }
        }
        let key = Rect::from_min_size(pos2(cell.left(), cell.bottom() - key_h), vec2(cell.width(), key_h));
        let can = !job.running() && i < job.prompts.len();
        if big_button(ui, key, "draw another", VALUE, false, can).clicked() {
            self.redraw(ctx, i);
        }
    }

    fn gallery_screen(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let avail_w = ui.available_width();
        let cols = ((avail_w / 230.0) as usize).max(1);
        let gap = 12.0;
        let cw = (avail_w - gap * (cols as f32 - 1.0)) / cols as f32;
        let mut cells: Vec<Option<Entry>> = vec![None];
        cells.extend(self.gallery.iter().cloned().map(Some));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(gap, gap);
            for chunk in cells.chunks(cols) {
                ui.horizontal_top(|ui| {
                    for cell in chunk {
                        ui.allocate_ui_with_layout(vec2(cw, cw * 1.2 + 74.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(cw);
                            ui.spacing_mut().item_spacing.y = 6.0;
                            match cell {
                                None => self.new_pages_card(ui, cw),
                                Some(entry) => self.gallery_card(ui, &ctx, entry, cw),
                            }
                        });
                    }
                });
            }
            if self.gallery.is_empty() {
                note(ui, "nothing here yet, make some pages first", LEGEND);
            }
        });
    }

    fn new_pages_card(&mut self, ui: &mut Ui, cw: f32) {
        let (rect, resp) = ui.allocate_exact_size(vec2(cw, cw * 1.2), Sense::click());
        let hot = resp.hovered();
        let p = ui.painter();
        p.rect_filled(rect, 2.0, if hot { BAND } else { WELL });
        p.rect_stroke(rect, 2.0, Stroke::new(if hot { 2.0 } else { 1.0 }, if hot { READOUT } else { ETCH }), egui::StrokeKind::Inside);
        let c = rect.center() - vec2(0.0, 16.0);
        let arm = cw * 0.14;
        let s = Stroke::new(6.0, READOUT);
        p.line_segment([c - vec2(arm, 0.0), c + vec2(arm, 0.0)], s);
        p.line_segment([c - vec2(0.0, arm), c + vec2(0.0, arm)], s);
        let g = egui_bench::text::lay(ui, egui_bench::text::caps("new pages", legend_font(16.0), READOUT, 2.0), cw - 20.0);
        ui.painter().galley(pos2(rect.center().x - g.size().x / 2.0, c.y + arm + 18.0), g, READOUT);
        if hot {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            self.go_home();
        }
        Line::new().value("start something new").size(12.0).elided(ui);
    }

    fn gallery_card(&mut self, ui: &mut Ui, ctx: &egui::Context, entry: &Entry, cw: f32) {
        let status = self.jobs.get(&entry.meta.id).map(|j| j.status).unwrap_or_else(|| entry.meta.status.clone());
        let (rect, resp) = ui.allocate_exact_size(vec2(cw, cw * 1.2), Sense::click());
        ui.painter().rect_filled(rect, 2.0, Color32::WHITE);
        if let Some((_, tex)) = self.thumbs.get(&entry.meta.id) {
            let img = Rect::from_center_size(rect.center(), vec2(cw * 0.85, cw * 0.85 * 1.414).min(rect.size()));
            ui.painter().image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        let edge = match &status {
            Status::Running => TRACE,
            Status::Failed(_) | Status::Interrupted => FAULT,
            Status::Done if resp.hovered() => READOUT,
            Status::Done => ETCH,
        };
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(if status == Status::Done { 1.0 } else { 2.0 }, edge), egui::StrokeKind::Inside);
        if status != Status::Done {
            self.card_state(ui, entry, &status, rect);
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            self.open_entry(ctx, entry);
        }
        Line::new().value(entry.meta.request.clone()).size(12.0).elided(ui);
        match status {
            Status::Running => {
                ui.add_enabled(false, Key::new("drawing").outline(TRACE).min_width(cw));
            }
            Status::Failed(_) | Status::Interrupted => {
                let half = (cw - 6.0) / 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if ui.add(Key::new("retry").solid(READOUT).min_width(half)).clicked() {
                        self.retry(ctx, &entry.meta.id);
                    }
                    if ui.add(Key::new("remove").outline(FAULT).min_width(half)).clicked() {
                        self.removing = Some(entry.meta.id.clone());
                    }
                });
            }
            Status::Done => {
                if ui.add(Key::new("remove").outline(FAULT).min_width(cw)).clicked() {
                    self.removing = Some(entry.meta.id.clone());
                }
            }
        }
    }

    fn card_state(&self, ui: &mut Ui, entry: &Entry, status: &Status, rect: Rect) {
        let marks = self.page_marks(entry);
        let ready = marks.iter().filter(|m| **m == Mark::Done).count();
        let band = Rect::from_min_size(pos2(rect.left() + 8.0, rect.bottom() - 74.0), vec2(rect.width() - 16.0, 66.0));
        let (label, tint) = match status {
            Status::Running if entry.meta.prompts.is_empty() && ready == 0 => ("thinking".to_string(), TRACE),
            Status::Running => (format!("drawing {ready} of {PAGES}"), TRACE),
            Status::Interrupted => ("interrupted".to_string(), FAULT),
            _ => ("could not finish".to_string(), FAULT),
        };
        let top = Rect::from_min_size(band.min, vec2(band.width(), 34.0));
        if *status == Status::Running {
            busy(ui, top, None, &label, tint);
        } else {
            let p = ui.painter();
            p.rect_filled(top, 2.0, WELL);
            p.rect_stroke(top, 2.0, Stroke::new(1.0, tint), egui::StrokeKind::Inside);
            let g = egui_bench::text::lay(ui, egui_bench::text::caps(&label, legend_font(13.0), tint, 1.0), top.width() - 10.0);
            ui.painter().galley(top.center() - g.size() / 2.0, g, tint);
        }
        let n = marks.len() as f32;
        let dw = ((band.width() - 6.0 * (n - 1.0)) / n).min(40.0);
        let start = band.center().x - (dw * n + 6.0 * (n - 1.0)) / 2.0;
        let t = ui.input(|i| i.time) as f32;
        for (i, mark) in marks.iter().enumerate() {
            let r = Rect::from_min_size(pos2(start + i as f32 * (dw + 6.0), band.bottom() - 22.0), vec2(dw, 22.0));
            let p = ui.painter();
            match mark {
                Mark::Done => {
                    p.rect_filled(r, 2.0, READOUT);
                    let c = r.center();
                    let tick = [c + vec2(-5.0, 0.0), c + vec2(-1.0, 4.0), c + vec2(6.0, -4.0)];
                    p.add(egui::Shape::line(tick.to_vec(), Stroke::new(2.0, WELL)));
                }
                Mark::Pending => {
                    let pulse = 0.35 + 0.35 * ((t * 3.0 + i as f32).sin() * 0.5 + 0.5);
                    p.rect_filled(r, 2.0, TRACE.gamma_multiply(pulse));
                    p.rect_stroke(r, 2.0, Stroke::new(1.0, TRACE), egui::StrokeKind::Inside);
                }
                Mark::Missing => {
                    p.rect_filled(r, 2.0, WELL);
                    p.rect_stroke(r, 2.0, Stroke::new(1.0, FAULT), egui::StrokeKind::Inside);
                }
            }
        }
        if *status == Status::Running {
            ui.ctx().request_repaint_after(Duration::from_millis(60));
        }
    }

    fn parent_button(&mut self, ui: &mut Ui, rect: Rect, font: f32) {
        let id = egui::Id::unique("parent-hold");
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        let down = resp.is_pointer_button_down_on();
        let held = match (down, self.hold) {
            (true, None) => {
                self.hold = Some(Instant::now());
                0.0
            }
            (true, Some(t)) => t.elapsed().as_secs_f32(),
            (false, _) => {
                self.hold = None;
                0.0
            }
        };
        let f = (held / HOLD_TO_OPEN).clamp(0.0, 1.0);
        let label = if down { "keep holding" } else { "parents" };
        sized_button(ui, rect, label, if down { READOUT } else { LEGEND }, false, true, font);
        if down {
            let mut fill = rect;
            fill.set_width(rect.width() * f);
            ui.painter().rect_filled(fill, 2.0, READOUT.gamma_multiply(0.3));
        }
        if down {
            ui.ctx().request_repaint();
        }
        if f >= 1.0 {
            self.hold = None;
            self.draft = Some(self.settings.clone());
        }
    }

    fn home(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let avail = ui.available_rect_before_wrap();
        let d = (avail.height() * 0.42).clamp(110.0, 300.0).min(avail.width() * 0.8);
        let lead = (avail.height() * 0.1).min(40.0);
        let centre = pos2(avail.center().x, avail.top() + lead + d / 2.0 + 10.0);
        let rect = Rect::from_center_size(centre, vec2(d, d));
        let listening = self.voice.listening();
        let reading = self.voice.reading();
        let id = egui::Id::unique("mic");
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        let mut down = resp.is_pointer_button_down_on();
        if let Some((since, secs)) = &mut self.auto_hold {
            if *secs > 0.0 {
                let t = *since.get_or_insert_with(Instant::now);
                if t.elapsed().as_secs_f32() < *secs {
                    down = true;
                } else {
                    *secs = 0.0;
                }
                ctx.request_repaint();
            }
        }
        if down && !self.voice.busy() {
            if let Err(e) = self.voice.start() {
                self.say(false, e);
            }
        }
        if listening && !down {
            let settings = self.voice_settings();
            let wake = ctx.clone();
            self.voice.finish(&settings, move || wake.request_repaint());
        }
        let level = self.voice.level();
        self.level = self.level.max(level).min(1.0) * 0.85 + level * 0.15;
        let p = ui.painter();
        let tint = if listening { FAULT } else if reading { TRACE } else { READOUT };
        let pulse = if listening { 1.0 + (self.level * 3.0).min(0.35) } else { 1.0 };
        p.circle_filled(centre, d / 2.0 * pulse + 10.0, tint.gamma_multiply(0.12));
        p.circle_stroke(centre, d / 2.0 * pulse + 10.0, Stroke::new(2.0, tint.gamma_multiply(0.5)));
        let fill = if down { tint.gamma_multiply(0.8) } else { tint };
        p.circle_filled(centre, d / 2.0, fill);
        paint_mic(p, centre, d / 2.0, WELL);
        if reading {
            paint_spinner(ui.painter(), centre, d / 2.0 + 10.0, ui.input(|i| i.time) as f32, TRACE);
        }
        if listening || reading {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let caption = if listening {
            format!("listening {:.0}s, let go when you are done", self.voice.seconds())
        } else {
            "hold and say what you want to colour".to_string()
        };
        let below = avail.top() + lead + d + 40.0;
        let cap_rect = Rect::from_min_size(pos2(avail.left(), below), vec2(avail.width(), 30.0));
        if reading {
            let w = avail.width().min(420.0);
            let bar = Rect::from_center_size(cap_rect.center(), vec2(w, 36.0));
            self.whisper_loader(ui, bar);
        } else {
            ui.scope_builder(egui::UiBuilder::new().max_rect(cap_rect), |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(caption.to_uppercase()).font(legend_font(15.0)).color(if listening { FAULT } else { LEGEND }).extra_letter_spacing(1.5));
                });
            });
        }
        let w = avail.width().min(620.0);
        let field_rect = Rect::from_min_size(pos2(avail.center().x - w / 2.0, below + 50.0), vec2(w, 44.0));
        let mut go = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(field_rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            let r = field_then(ui, &mut self.request, "or type it here", 120.0, |ui| {
                go = ui.add(Key::new("draw").solid(READOUT).min_width(100.0)).clicked();
            });
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
        });
        if go {
            let text = self.request.clone();
            self.start(&ctx, text);
        }
        let detail_top = field_rect.bottom() + 16.0;
        self.detail_row(ui, pos2(avail.center().x, detail_top));
        self.reference_row(ui, &ctx, pos2(avail.center().x, detail_top + DETAIL_ROW_H + 16.0));
    }

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save(&settings::settings_path()) {
            self.say(false, format!("could not save settings: {e}"));
        }
    }

    pub fn set_detail(&mut self, detail: Detail) {
        if self.settings.detail != detail {
            self.settings.detail = detail;
            self.save_settings();
        }
    }

    fn detail_row(&mut self, ui: &mut Ui, top: Pos2) {
        let cell = vec2(64.0, 40.0);
        let gap = 8.0;
        let total = cell.x * 5.0 + gap * 4.0;
        let left = top.x - total / 2.0;
        let current = self.settings.detail;
        let caption = Rect::from_min_size(pos2(left, top.y), vec2(total, 18.0));
        let g = egui_bench::text::lay(ui, egui_bench::text::caps(&format!("how detailed: {}", current.label()), legend_font(13.0), LEGEND, 1.5), total);
        ui.painter().galley(pos2(caption.center().x - g.size().x / 2.0, caption.top()), g, LEGEND);
        for (i, level) in Detail::ALL.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(left + i as f32 * (cell.x + gap), top.y + 22.0), cell);
            let id = egui::Id::unique(("detail", i));
            let resp = ui.interact(rect, id, Sense::click());
            let picked = i <= current.index();
            let exact = *level == current;
            let (fill, edge, ink) = if picked { (READOUT.gamma_multiply(if exact { 0.9 } else { 0.35 }), READOUT, if exact { WELL } else { READOUT }) } else { (WELL, if hot(&resp, true) { VALUE } else { ETCH }, LEGEND) };
            let p = ui.painter();
            p.rect_filled(rect, 2.0, fill);
            p.rect_stroke(rect, 2.0, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
            let lines = i + 1;
            let span = rect.height() - 12.0;
            for k in 0..lines {
                let y = rect.top() + 6.0 + span * (k as f32 + 0.5) / lines as f32;
                p.line_segment([pos2(rect.left() + 12.0, y), pos2(rect.right() - 12.0, y)], Stroke::new(if lines > 3 { 1.5 } else { 3.0 }, ink));
            }
            if hot(&resp, true) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                self.set_detail(*level);
            }
        }
    }

    fn reference_row(&mut self, ui: &mut Ui, ctx: &egui::Context, top: Pos2) {
        if !crate::platform::CAN_PICK_PICTURE && self.reference.is_none() {
            return;
        }
        match self.reference.clone() {
            Some((_, tex)) => {
                let thumb = Rect::from_min_size(pos2(top.x - 170.0, top.y), vec2(64.0, 64.0));
                ui.painter().rect_filled(thumb, 2.0, Color32::WHITE);
                let size = tex.size_vec2();
                let fit = (64.0 / size.x).min(64.0 / size.y);
                ui.painter().image(
                    tex.id(),
                    Rect::from_center_size(thumb.center(), size * fit),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                ui.painter().rect_stroke(thumb, 2.0, Stroke::new(2.0, READOUT), egui::StrokeKind::Outside);
                let text = Rect::from_min_size(pos2(thumb.right() + 14.0, top.y), vec2(150.0, 64.0));
                let g = egui_bench::text::lay(ui, egui_bench::text::caps("drawing from this picture", legend_font(12.0), READOUT, 1.0), 150.0);
                ui.painter().galley(pos2(text.left(), text.center().y - g.size().y / 2.0), g, READOUT);
                let key = Rect::from_min_size(pos2(text.right() + 10.0, top.y), vec2(110.0, 30.0));
                if sized_button(ui, key, "take off", FAULT, false, true, 13.0).clicked() {
                    self.reference = None;
                }
                let close = self.settings.close;
                let mode = Rect::from_min_size(pos2(text.right() + 10.0, top.y + 34.0), vec2(150.0, 30.0));
                if sized_button(ui, mode, if close { "stay close" } else { "just inspired" }, VALUE, false, true, 13.0).clicked() {
                    self.settings.close = !close;
                    self.save_settings();
                }
            }
            None => {
                let key = Rect::from_min_size(pos2(top.x - 140.0, top.y), vec2(280.0, 44.0));
                if big_button(ui, key, "add a picture", VALUE, false, true).clicked() {
                    self.pick_reference(ctx);
                }
            }
        }
    }

    fn modals(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.removing.clone() {
            let (mut remove, mut keep) = (false, false);
            let m = egui::Modal::new(egui::Id::unique("remove-set"))
                .backdrop_color(Color32::from_black_alpha(150))
                .show(ctx, |ui| {
                    ui.set_width(380.0);
                    modal_title(ui, "Delete these pages?");
                    note(ui, "They are taken off this device. Pages already printed or saved stay.", LEGEND);
                    footer(ui, |ui| {
                        if ui.add(Key::new("delete").solid(FAULT).min_width(100.0)).clicked() {
                            remove = true;
                        }
                        if ui.add(Key::new("keep them").outline(VALUE).min_width(100.0)).clicked() {
                            keep = true;
                        }
                    });
                });
            if remove {
                if let Err(e) = self.store.remove(&id) {
                    self.say(false, e);
                }
                self.open_gallery(ctx);
                self.removing = None;
            }
            if keep || m.should_close() {
                self.removing = None;
            }
        }
        let Some(mut draft) = self.draft.take() else { return };
        let (mut save, mut close) = (false, false);
        let r = egui::Modal::new(egui::Id::unique("settings"))
            .backdrop_color(Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(520.0f32.min(ctx.content_rect().width() - 40.0));
                modal_title(ui, "Settings for parents");
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .max_height((ctx.content_rect().height() * 0.65).clamp(240.0, 700.0))
                    .show(ui, |ui| settings_form(ui, &mut draft));
                footer(ui, |ui| {
                    if ui.add(Key::new("save").solid(READOUT).min_width(100.0)).clicked() {
                        save = true;
                    }
                    if ui.add(Key::new("cancel").outline(VALUE).min_width(100.0)).clicked() {
                        close = true;
                    }
                });
            });
        if save {
            match draft.save(&settings::settings_path()) {
                Ok(()) => {
                    self.settings = draft;
                    self.say(true, "settings saved");
                }
                Err(e) => {
                    self.say(false, format!("could not save settings: {e}"));
                    self.draft = Some(draft);
                }
            }
        } else if close || r.should_close() {
        } else {
            self.draft = Some(draft);
        }
    }
}

fn settings_form(ui: &mut Ui, s: &mut Settings) {
    section(ui, "pictures", "who draws the pages", |ui| {
        let mut kind = s.kind();
        row(ui, "drawn by", |ui| {
            if choice(ui, "backend", &mut kind, [(Kind::Comfy, Kind::Comfy.label().to_string()), (Kind::OpenRouter, Kind::OpenRouter.label().to_string())]) {
                s.backend = kind.id().into();
            }
        });
        match kind {
            Kind::Comfy => row(ui, "comfyui", |ui| {
                field(ui, &mut s.comfy_url, "http://10.100.2.32:8188");
            }),
            Kind::OpenRouter => {
                row(ui, "address", |ui| {
                    field(ui, &mut s.openrouter_url, "https://openrouter.ai/api/v1");
                });
                row(ui, "key", |ui| {
                    secret(ui, &mut s.openrouter_key);
                });
                row(ui, "model", |ui| {
                    field(ui, &mut s.openrouter_model, crate::backend::openrouter_default_model());
                });
            }
        }
        let mut paper = s.paper();
        row(ui, "paper", |ui| {
            if choice(ui, "paper", &mut paper, [(Paper::A4, "a4".to_string()), (Paper::Letter, "letter".to_string())]) {
                s.paper = paper.id().into();
            }
        });
    });
    ui.add_space(8.0);
    section(ui, "ideas", "the language model that plans the pages", |ui| {
        row(ui, "address", |ui| {
            field(ui, &mut s.llm_url, "https://yalr.v0l.io/v1");
        });
        row(ui, "key", |ui| {
            secret(ui, &mut s.llm_key);
        });
        row(ui, "model", |ui| {
            field(ui, &mut s.llm_model, "code");
        });
        row(ui, "sees pictures", |ui| {
            field(ui, &mut s.vision_model, "same as model");
        });
    });
    ui.add_space(8.0);
    section(ui, "listening", "turns speech into words", |ui| {
        let mut here = s.listen;
        row(ui, "listen", |ui| {
            if choice(ui, "listen", &mut here, [(Listen::Here, "on this device".to_string()), (Listen::Server, "on a speech server".to_string())]) {
                s.listen = here;
            }
        });
        if s.listen == Listen::Here {
            row(ui, "model", |ui| {
                choice(ui, "whisper", &mut s.whisper_model, egui_bench::voice::LOCAL_MODELS.iter().map(|m| (m.0.to_string(), format!("{} ({} MB)", m.2, m.3))));
            });
        } else {
            row(ui, "address", |ui| {
                field(ui, &mut s.speech_url, "http://host:7420/v1");
            });
            row(ui, "key", |ui| {
                secret(ui, &mut s.speech_key);
            });
            row(ui, "model", |ui| {
                field(ui, &mut s.speech_model, "whisper-1");
            });
        }
    });
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if !cfg!(target_os = "android") {
            return;
        }
        let keyboard = crate::system::keyboard_height(ctx);
        let bottom = keyboard.max(crate::system::bottom_inset(ctx));
        raw.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 {
            left: 0.0,
            right: 0.0,
            top: crate::system::top_inset(ctx),
            bottom,
        }));
        if keyboard > 0.0 || ctx.egui_wants_keyboard_input() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        let above = ctx.content_rect().top() - ctx.viewport_rect().top();
        let below = ctx.viewport_rect().bottom() - ctx.content_rect().bottom();
        let below = if below > 0.0 { below + 8.0 } else { below };
        if below > 0.0 {
            egui::Panel::bottom("system-bottom")
                .exact_size(below)
                .resizable(false)
                .frame(egui::Frame::NONE.fill(CHASSIS))
                .show(ui, |_| {});
        }
        egui::Panel::top("header")
            .frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin {
                left: 14,
                right: 14,
                top: 10 + above as i8,
                bottom: 8,
            }))
            .show(ui, |ui| self.header(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let r = ui.max_rect();
            ui.painter().rect_filled(r, 0.0, CHASSIS);
            ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, ETCH));
            egui::Frame::NONE.inner_margin(egui::Margin::same(14)).show(ui, |ui| match self.screen {
                Screen::Home => self.home(ui),
                Screen::Job => self.job_screen(ui),
                Screen::Gallery => self.gallery_screen(ui),
            });
        });
        if let Some((ok, text)) = self.notice.clone() {
            let tone = if ok { Tone::Ok } else { Tone::Fault };
            if toast(&ctx, "notice", &text, tone) {
                self.notice = None;
            }
        }
        self.modals(&ctx);
        if self.notice_until.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Fake, Reply, serve};
    use crate::jobs::tests::{Gate, thinker};
    use base64::Engine;
    use serde_json::json;

    fn painter(gate: Arc<Gate>) -> Fake {
        let img = GrayImage::from_fn(300, 400, |x, y| {
            let d = (((x as f32 - 150.0).powi(2) + (y as f32 - 200.0).powi(2)).sqrt() - 90.0).abs();
            image::Luma([if d < 4.0 { 30 } else { 235 }])
        });
        let png = base64::engine::general_purpose::STANDARD.encode(crate::clean::encode_png(&img).unwrap());
        serve(move |_| {
            gate.wait_open();
            let url = format!("data:image/png;base64,{png}");
            Reply::json(json!({"choices":[{"message":{"images":[{"image_url":{"url":url}}]}}]}))
        })
    }

    struct Rig {
        _dir: tempfile::TempDir,
        _thinker: Fake,
        _painter: Fake,
        gate: Arc<Gate>,
        ctx: egui::Context,
        store: Store,
        app: App,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let (thinker, llm) = thinker();
        let gate = Gate::shut();
        let painter = painter(gate.clone());
        let settings = Settings {
            llm_url: llm.url,
            backend: Kind::OpenRouter.id().into(),
            openrouter_url: painter.url.clone(),
            openrouter_key: "k".into(),
            ..Settings::default()
        };
        let ctx = egui::Context::default();
        let store = Store::new(dir.path());
        let app = App::with(&ctx, settings, store.clone());
        Rig { _dir: dir, _thinker: thinker, _painter: painter, gate, ctx, store, app }
    }

    fn frame(rig: &mut Rig) {
        frame_with(rig, Vec::new());
    }

    fn frame_with(rig: &mut Rig, events: Vec<egui::Event>) {
        let ctx = rig.ctx.clone();
        let app = &mut rig.app;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            app.poll(ui.ctx());
            match app.screen {
                Screen::Home => app.home(ui),
                Screen::Job => app.job_screen(ui),
                Screen::Gallery => app.gallery_screen(ui),
            }
        });
        out.textures_delta.clear();
    }

    fn settle(rig: &mut Rig, what: &str, done: impl Fn(&App) -> bool) {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            frame(rig);
            if done(&rig.app) {
                return;
            }
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn leaving_the_generating_view_keeps_the_job_and_coming_back_shows_its_progress() {
        let mut r = rig();
        let ctx = r.ctx.clone();
        r.app.start(&ctx, "cats".into());
        let id = r.app.current().unwrap().id.clone();
        settle(&mut r, "the prompts", |a| a.current().is_some_and(|j| j.prompts.len() == PAGES));
        r.app.go_home();
        assert_eq!(r.app.screen(), Screen::Home);
        r.app.open_gallery(&ctx);
        frame(&mut r);
        let entry = r.app.gallery().iter().find(|e| e.meta.id == id).cloned().unwrap();
        r.gate.open();
        r.app.open_entry(&ctx, &entry);
        assert_eq!(r.app.screen(), Screen::Job);
        assert_eq!(r.app.current().unwrap().id, id);
        settle(&mut r, "every page", |a| a.current().is_some_and(|j| j.ready() == PAGES && !j.running()));
        assert!(r.app.view.as_ref().unwrap().textures.iter().all(Option::is_some));
    }

    #[test]
    fn a_request_still_drawing_is_listed_in_my_pages_with_its_page_progress() {
        let mut r = rig();
        let ctx = r.ctx.clone();
        r.app.start(&ctx, "dogs".into());
        let id = r.app.current().unwrap().id.clone();
        r.app.open_gallery(&ctx);
        settle(&mut r, "the gallery to list the job", |a| a.gallery().iter().any(|e| e.meta.id == id));
        let entry = r.app.gallery().iter().find(|e| e.meta.id == id).cloned().unwrap();
        assert_eq!(entry.meta.status, Status::Running);
        settle(&mut r, "the prompts", |a| a.jobs.get(&id).is_some_and(|j| j.prompts.len() == PAGES));
        assert_eq!(r.app.page_marks(&entry), vec![Mark::Pending; PAGES]);
        r.gate.open();
        settle(&mut r, "the job to finish", |a| !a.jobs.running(&id));
    }

    #[test]
    fn a_finished_job_turns_into_a_normal_my_pages_entry() {
        let mut r = rig();
        let ctx = r.ctx.clone();
        r.gate.open();
        r.app.start(&ctx, "owls".into());
        let id = r.app.current().unwrap().id.clone();
        r.app.open_gallery(&ctx);
        settle(&mut r, "the entry to be done", |a| {
            a.gallery().iter().any(|e| e.meta.id == id && e.meta.status == Status::Done)
        });
        let entry = r.store.get(&id).unwrap();
        assert!((0..PAGES).all(|i| entry.page_path(i).exists()));
        assert_eq!(r.app.page_marks(&entry), vec![Mark::Done; PAGES]);
        assert!(r.app.thumbs.contains_key(&id));
    }

    #[test]
    fn a_run_cut_short_by_closing_the_app_shows_interrupted_and_retry_finishes_it() {
        let mut r = rig();
        let ctx = r.ctx.clone();
        r.app.start(&ctx, "fish".into());
        let id = r.app.current().unwrap().id.clone();
        settle(&mut r, "the prompts", |a| a.current().is_some_and(|j| j.prompts.len() == PAGES));
        let fresh = Gate::shut();
        let painter = painter(fresh.clone());
        let mut settings = r.app.settings.clone();
        settings.openrouter_url = painter.url.clone();
        r.app = App::with(&ctx, settings, r.store.clone());
        r.app.open_gallery(&ctx);
        frame(&mut r);
        let entry = r.app.gallery().iter().find(|e| e.meta.id == id).cloned().unwrap();
        assert_eq!(entry.meta.status, Status::Interrupted);
        fresh.open();
        r.app.retry(&ctx, &id);
        settle(&mut r, "the retry to finish", |a| {
            a.gallery().iter().any(|e| e.meta.id == id && e.meta.status == Status::Done)
        });
    }

    #[test]
    fn my_pages_has_a_new_pages_button_that_goes_home() {
        let mut r = rig();
        let ctx = r.ctx.clone();
        r.app.open_gallery(&ctx);
        frame(&mut r);
        r.app.request = "old words".into();
        let at = pos2(60.0, 60.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame_with(&mut r, vec![egui::Event::PointerMoved(at)]);
        frame_with(&mut r, vec![button(true)]);
        frame_with(&mut r, vec![button(false)]);
        assert_eq!(r.app.screen(), Screen::Home);
        assert!(r.app.request.is_empty());
    }

    #[test]
    fn a_picked_reference_picture_goes_to_the_image_model_and_is_used_once() {
        let mut r = rig();
        let ctx = r.ctx.clone();
        r.gate.open();
        let photo = GrayImage::from_fn(2400, 1800, |x, _| image::Luma([(x % 255) as u8]));
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(photo).write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
        let png = reference_png(jpeg.get_ref()).unwrap();
        let small = image::load_from_memory(&png).unwrap();
        assert!(small.width() <= 1024 && small.height() <= 1024);
        r.app.use_reference(&ctx, png.clone());
        r.app.start(&ctx, "my dog".into());
        assert!(r.app.reference.is_none());
        settle(&mut r, "every page", |a| a.current().is_some_and(|j| !j.running()));
        let drawn: Vec<_> = r._painter.requests().into_iter().filter(|s| s.text().contains("modalities")).collect();
        assert_eq!(drawn.len(), PAGES);
        let sent = base64::engine::general_purpose::STANDARD.encode(&png);
        assert!(drawn.iter().all(|s| s.text().contains(&sent)));
    }

    #[test]
    fn a_file_that_is_not_a_picture_is_turned_down() {
        assert!(reference_png(b"not a picture").unwrap_err().contains("could not be read"));
    }

    #[test]
    fn new_pages_start_with_nothing_picked_for_printing() {
        let mut r = rig();
        let ctx = r.ctx.clone();
        r.gate.open();
        r.app.start(&ctx, "bees".into());
        settle(&mut r, "every page", |a| a.current().is_some_and(|j| j.ready() == PAGES));
        assert_eq!(r.app.selected, vec![false; PAGES]);
    }
}
