use crate::backend::Kind;
use crate::clean::Paper;
use crate::pipeline::{self, Event};
use crate::plan::{PAGES, Plan};
use crate::settings::{self, Listen, Settings};
use crate::store::{self, Entry, Meta, Store};
use crate::voice::{self, Said, Voice};
use crate::{clean, net, outputs};
use egui::{Color32, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Ui, pos2, vec2};
use egui_bench::prelude::*;
use image::GrayImage;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

const HOLD_TO_OPEN: f32 = 1.5;
const BIG: f32 = 64.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Home,
    Making,
    Results,
    Gallery,
}

enum Slot {
    Empty,
    Pending,
    Ready { img: GrayImage, tex: TextureHandle },
    Failed(String),
}

enum Msg {
    Pipeline(u64, Event),
    Notice(bool, String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Export {
    Print,
    Pdf,
    Png,
}

pub struct App {
    settings: Settings,
    store: Store,
    voice: Voice,
    screen: Screen,
    request: String,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    run: u64,
    running: bool,
    slots: Vec<Slot>,
    selected: Vec<bool>,
    plan: Option<Plan>,
    prompts: Vec<String>,
    set_id: Option<String>,
    set_made: u64,
    paper: Paper,
    step: String,
    notice: Option<(bool, String)>,
    notice_until: Option<Instant>,
    draft: Option<Settings>,
    hold: Option<Instant>,
    gallery: Vec<Entry>,
    thumbs: HashMap<String, TextureHandle>,
    removing: Option<String>,
    level: f32,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        egui_bench::install(&cc.egui_ctx);
        let settings = Settings::load(&settings::settings_path());
        let (tx, rx) = channel();
        let mut app = App {
            store: Store::new(&settings::data_dir()),
            paper: settings.paper(),
            settings,
            voice: Voice::default(),
            screen: Screen::Home,
            request: String::new(),
            tx,
            rx,
            run: 0,
            running: false,
            slots: (0..PAGES).map(|_| Slot::Empty).collect(),
            selected: vec![true; PAGES],
            plan: None,
            prompts: Vec::new(),
            set_id: None,
            set_made: 0,
            step: String::new(),
            notice: None,
            notice_until: None,
            draft: None,
            hold: None,
            gallery: Vec::new(),
            thumbs: HashMap::new(),
            removing: None,
            level: 0.0,
        };
        app.apply_env(&cc.egui_ctx);
        app
    }

    fn apply_env(&mut self, ctx: &egui::Context) {
        match std::env::var("COLOURING_SHOW").as_deref() {
            Ok("settings") => self.draft = Some(self.settings.clone()),
            Ok("gallery") => self.open_gallery(ctx),
            Ok("results") => {
                if let Some(entry) = self.store.list().first().cloned() {
                    self.open_set(ctx, &entry);
                }
            }
            _ => {}
        }
        if let Ok(text) = std::env::var("COLOURING_AUTORUN") {
            if !text.trim().is_empty() {
                self.request = text.clone();
                self.start(ctx, text);
            }
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

    fn start(&mut self, ctx: &egui::Context, request: String) {
        let request = request.trim().to_string();
        if request.is_empty() || self.running {
            return;
        }
        if let Some(problem) = self.settings.problem() {
            self.say(false, problem);
            return;
        }
        self.run += 1;
        self.running = true;
        self.screen = Screen::Making;
        self.step = "thinking".into();
        self.slots = (0..PAGES).map(|_| Slot::Empty).collect();
        self.selected = vec![true; PAGES];
        self.plan = None;
        self.prompts.clear();
        self.set_id = None;
        self.paper = self.settings.paper();
        let (run, tx, ctx2) = (self.run, self.tx.clone(), ctx.clone());
        let sink: pipeline::Sink = Arc::new(move |e| {
            let _ = tx.send(Msg::Pipeline(run, e));
            ctx2.request_repaint();
        });
        let backend: Arc<dyn crate::backend::ImageBackend> = Arc::from(self.settings.image_backend());
        net::spawn(pipeline::run(self.settings.llm(), backend, self.paper, request, sink));
    }

    fn redraw(&mut self, ctx: &egui::Context, index: usize) {
        let Some(prompt) = self.prompts.get(index).cloned() else { return };
        if self.running {
            return;
        }
        self.run += 1;
        self.running = true;
        self.slots[index] = Slot::Pending;
        let (run, tx, ctx2) = (self.run, self.tx.clone(), ctx.clone());
        let sink: pipeline::Sink = Arc::new(move |e| {
            let _ = tx.send(Msg::Pipeline(run, e));
            ctx2.request_repaint();
        });
        let backend: Arc<dyn crate::backend::ImageBackend> = Arc::from(self.settings.image_backend());
        net::spawn(pipeline::redraw(backend, self.paper, index, prompt, sink));
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

    fn meta(&self) -> Option<Meta> {
        Some(Meta {
            id: self.set_id.clone()?,
            request: self.request.trim().to_string(),
            made: self.set_made,
            paper: self.paper.id().into(),
            plan: self.plan.clone(),
            prompts: self.prompts.clone(),
            pages: PAGES,
        })
    }

    fn handle(&mut self, ctx: &egui::Context, event: Event) {
        match event {
            Event::Planning => self.step = "thinking".into(),
            Event::Planned(plan) => {
                self.step = "drawing".into();
                self.plan = Some(plan);
            }
            Event::Drawing(prompts) => {
                self.prompts = prompts;
                self.set_made = net::unix_seconds();
                self.set_id = Some(store::new_id(self.set_made));
                if let Some(meta) = self.meta() {
                    if let Err(e) = self.store.save_meta(&meta) {
                        self.say(false, format!("could not keep these pages: {e}"));
                    }
                }
                self.slots = (0..PAGES).map(|_| Slot::Pending).collect();
                self.screen = Screen::Results;
            }
            Event::Page(i, img) => {
                if let (Some(id), Ok(png)) = (self.set_id.clone(), clean::encode_png(&img)) {
                    let _ = self.store.save_page(&id, i, &png);
                }
                let tex = Self::texture(ctx, &format!("page-{}-{}", self.run, i), &img, 700);
                if i < self.slots.len() {
                    self.slots[i] = Slot::Ready { img, tex };
                }
            }
            Event::PageFailed(i, why) => {
                if i < self.slots.len() {
                    self.slots[i] = Slot::Failed(why);
                }
            }
            Event::Refused(why) => {
                self.running = false;
                self.screen = Screen::Home;
                self.say(false, why);
            }
            Event::Failed(why) => {
                self.running = false;
                self.screen = Screen::Home;
                self.say(false, why);
            }
            Event::Finished => {
                self.running = false;
                if matches!(self.screen, Screen::Making) {
                    self.screen = Screen::Home;
                }
                if self.slots.iter().all(|s| matches!(s, Slot::Failed(_))) && !self.slots.is_empty() {
                    self.say(false, "no page could be drawn, check the picture maker in settings");
                }
            }
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Pipeline(run, event) if run == self.run => self.handle(ctx, event),
                Msg::Pipeline(..) => {}
                Msg::Notice(ok, text) => self.say(ok, text),
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
        if let Some(until) = self.notice_until {
            if Instant::now() > until {
                self.notice = None;
                self.notice_until = None;
            }
        }
    }

    fn open_gallery(&mut self, ctx: &egui::Context) {
        self.gallery = self.store.list();
        self.thumbs.clear();
        for entry in &self.gallery {
            if let Ok(img) = image::open(entry.page_path(0)) {
                let tex = Self::texture(ctx, &format!("thumb-{}", entry.meta.id), &img.to_luma8(), 360);
                self.thumbs.insert(entry.meta.id.clone(), tex);
            }
        }
        self.screen = Screen::Gallery;
    }

    fn open_set(&mut self, ctx: &egui::Context, entry: &Entry) {
        self.run += 1;
        self.running = false;
        self.slots = (0..PAGES).map(|_| Slot::Empty).collect();
        for i in 0..entry.meta.pages.min(PAGES) {
            self.slots[i] = match image::open(entry.page_path(i)) {
                Ok(img) => {
                    let img = img.to_luma8();
                    let tex = Self::texture(ctx, &format!("set-{}-{}", entry.meta.id, i), &img, 700);
                    Slot::Ready { img, tex }
                }
                Err(e) => Slot::Failed(e.to_string()),
            };
        }
        self.selected = vec![true; PAGES];
        self.plan = entry.meta.plan.clone();
        self.prompts = entry.meta.prompts.clone();
        self.set_id = Some(entry.meta.id.clone());
        self.set_made = entry.meta.made;
        self.request = entry.meta.request.clone();
        self.paper = entry.paper();
        self.screen = Screen::Results;
    }

    fn export(&mut self, ctx: &egui::Context, kind: Export) {
        let pages: Vec<GrayImage> = self
            .slots
            .iter()
            .zip(&self.selected)
            .filter_map(|(s, on)| match s {
                Slot::Ready { img, .. } if *on => Some(img.clone()),
                _ => None,
            })
            .collect();
        if pages.is_empty() {
            self.say(false, "tap the pages you want first");
            return;
        }
        let (paper, name, tx, ctx) = (self.paper, outputs::slug(&self.request), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let refs: Vec<&GrayImage> = pages.iter().collect();
            let done = match kind {
                Export::Print => outputs::print(&refs, paper, &name).map(|_| "sent to the printer".to_string()),
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
    let job = egui_bench::text::caps(label, legend_font(19.0), ink, 2.0);
    let galley = egui_bench::text::lay(ui, job, rect.width() - 12.0);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ink);
    if hot(&resp, enabled) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
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
        ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new("COLOURING PAGES").font(legend_font(18.0)).color(VALUE).extra_letter_spacing(3.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(130.0, 40.0), Sense::hover());
                self.parent_button(ui, rect);
                let (rect, _) = ui.allocate_exact_size(vec2(130.0, 40.0), Sense::hover());
                let label = if self.screen == Screen::Gallery { "back" } else { "my pages" };
                if big_button(ui, rect, label, VALUE, false, true).clicked() {
                    if self.screen == Screen::Gallery {
                        self.screen = if self.slots.iter().any(|s| !matches!(s, Slot::Empty)) { Screen::Results } else { Screen::Home };
                    } else if !self.running || self.screen != Screen::Making {
                        self.open_gallery(&ctx);
                    }
                }
            });
        });
    }

    fn parent_button(&mut self, ui: &mut Ui, rect: Rect) {
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
        big_button(ui, rect, label, if down { READOUT } else { LEGEND }, false, true);
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
        let d = (avail.height() * 0.42).clamp(150.0, 300.0).min(avail.width() * 0.8);
        let centre = pos2(avail.center().x, avail.top() + avail.height() * 0.1 + d / 2.0 + 10.0);
        let rect = Rect::from_center_size(centre, vec2(d, d));
        let listening = self.voice.listening();
        let reading = self.voice.reading();
        let id = egui::Id::unique("mic");
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        let down = resp.is_pointer_button_down_on();
        if down && !self.voice.busy() && !self.running {
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
        if listening || reading {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let caption = if listening {
            format!("listening {:.0}s, let go when you are done", self.voice.seconds())
        } else if reading {
            if self.voice.status.is_empty() { "listening".into() } else { self.voice.status.clone() }
        } else {
            "hold and say what you want to colour".to_string()
        };
        let below = avail.top() + avail.height() * 0.1 + d + 40.0;
        let cap_rect = Rect::from_min_size(pos2(avail.left(), below), vec2(avail.width(), 30.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(cap_rect), |ui| {
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(caption.to_uppercase()).font(legend_font(15.0)).color(if listening { FAULT } else { LEGEND }).extra_letter_spacing(1.5));
            });
        });
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
    }

    fn making(&mut self, ui: &mut Ui) {
        let avail = ui.available_rect_before_wrap();
        let w = avail.width().min(560.0);
        let rect = Rect::from_center_size(avail.center(), vec2(w, 90.0));
        let label = format!("{}...", self.step);
        busy(ui, rect, None, &label, TRACE);
        let cap = Rect::from_min_size(pos2(rect.left(), rect.bottom() + 16.0), vec2(w, 30.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(cap), |ui| {
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(self.request.trim()).font(legend_font(16.0)).color(VALUE));
            });
        });
    }

    fn tile(&mut self, ui: &mut Ui, ctx: &egui::Context, i: usize, cell: Rect) {
        let key_h = 52.0;
        let area = Rect::from_min_max(cell.min, pos2(cell.max.x, cell.max.y - key_h - 8.0));
        let (pw, ph) = self.paper.pixels();
        let aspect = pw as f32 / ph as f32;
        let (mut w, mut h) = (area.width(), area.width() / aspect);
        if h > area.height() {
            h = area.height();
            w = h * aspect;
        }
        let page = Rect::from_center_size(area.center(), vec2(w.max(10.0), h.max(10.0)));
        let on = self.selected[i];
        let id = egui::Id::unique(("tile", i));
        let ready = matches!(self.slots[i], Slot::Ready { .. });
        let resp = ui.interact(page, id, if ready { Sense::click() } else { Sense::hover() });
        if resp.clicked() {
            self.selected[i] = !on;
        }
        match &self.slots[i] {
            Slot::Ready { tex, .. } => {
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
            Slot::Pending => {
                busy(ui, page, None, "drawing", TRACE);
            }
            Slot::Failed(why) => {
                let p = ui.painter();
                p.rect_filled(page, 2.0, WELL);
                p.rect_stroke(page, 2.0, Stroke::new(1.0, FAULT), egui::StrokeKind::Inside);
                let job = egui_bench::text::caps("could not draw this one", legend_font(13.0), FAULT, 1.0);
                let g = egui_bench::text::lay(ui, job, page.width() - 24.0);
                ui.painter().galley(page.center() - g.size() / 2.0 - vec2(0.0, 20.0), g, FAULT);
                let _ = why;
            }
            Slot::Empty => {}
        }
        let key = Rect::from_min_size(pos2(cell.left(), cell.bottom() - key_h), vec2(cell.width(), key_h));
        let can = !matches!(self.slots[i], Slot::Pending | Slot::Empty) && !self.running && !self.prompts.is_empty();
        if big_button(ui, key, "draw another", VALUE, false, can).clicked() {
            self.redraw(ctx, i);
        }
    }

    fn results(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let avail = ui.available_rect_before_wrap();
        let bar_h = BIG;
        let grid = Rect::from_min_max(avail.min, pos2(avail.max.x, avail.max.y - bar_h - 12.0));
        let landscape = grid.width() > grid.height() * 1.3;
        let (cols, rows) = if landscape { (4usize, 1usize) } else { (2, 2) };
        let gap = 14.0;
        let cw = (grid.width() - gap * (cols as f32 - 1.0)) / cols as f32;
        let ch = (grid.height() - gap * (rows as f32 - 1.0)) / rows as f32;
        for i in 0..PAGES {
            let (r, c) = (i / cols, i % cols);
            let cell = Rect::from_min_size(grid.min + vec2(c as f32 * (cw + gap), r as f32 * (ch + gap)), vec2(cw, ch));
            self.tile(ui, &ctx, i, cell);
        }
        let any = self.slots.iter().zip(&self.selected).any(|(s, on)| *on && matches!(s, Slot::Ready { .. }));
        let bar = Rect::from_min_max(pos2(avail.left(), avail.bottom() - bar_h), avail.max);
        let n = 4.0;
        let bw = (bar.width() - gap * (n - 1.0)) / n;
        let at = |k: f32| Rect::from_min_size(pos2(bar.left() + k * (bw + gap), bar.top()), vec2(bw, bar.height()));
        if big_button(ui, at(0.0), "new pages", VALUE, false, !self.running).clicked() {
            self.screen = Screen::Home;
            self.request.clear();
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

    fn gallery_screen(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if self.gallery.is_empty() {
            note(ui, "nothing here yet, make some pages first", LEGEND);
            return;
        }
        let avail_w = ui.available_width();
        let cols = ((avail_w / 230.0) as usize).max(1);
        let gap = 12.0;
        let cw = (avail_w - gap * (cols as f32 - 1.0)) / cols as f32;
        let entries = self.gallery.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(gap, gap);
            for chunk in entries.chunks(cols) {
                ui.horizontal_top(|ui| {
                    for entry in chunk {
                        ui.allocate_ui(vec2(cw, cw * 1.45), |ui| {
                            ui.set_width(cw);
                            let (rect, resp) = ui.allocate_exact_size(vec2(cw, cw * 1.2), Sense::click());
                            ui.painter().rect_filled(rect, 2.0, Color32::WHITE);
                            if let Some(tex) = self.thumbs.get(&entry.meta.id) {
                                let img = Rect::from_center_size(rect.center(), vec2(cw * 0.85, cw * 0.85 * 1.414).min(rect.size()));
                                ui.painter().image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, if resp.hovered() { READOUT } else { ETCH }), egui::StrokeKind::Inside);
                            if resp.clicked() {
                                self.open_set(&ctx, entry);
                            }
                            ui.horizontal(|ui| {
                                Line::new().value(entry.meta.request.clone()).size(12.0).elided(ui);
                            });
                            if ui.add(Key::new("remove").outline(FAULT).min_width(cw)).clicked() {
                                self.removing = Some(entry.meta.id.clone());
                            }
                        });
                    }
                });
            }
        });
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
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        let above = ctx.content_rect().top() - ctx.viewport_rect().top();
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
                Screen::Making => self.making(ui),
                Screen::Results => self.results(ui),
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
