use super::{MAX_SECONDS, Settings, judge};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use web_time_shim::Instant;

mod web_time_shim {
    pub use std::time::Instant;
}

#[derive(Clone, Debug, PartialEq)]
pub enum Said {
    Words(String),
    Failed(String),
}

struct Take {
    _stream: cpal::Stream,
    heard: Arc<Mutex<Vec<f32>>>,
    peak: Arc<AtomicU32>,
    rate: u32,
    since: Instant,
}

enum State {
    Idle,
    Listening(Box<Take>),
    Reading,
}

#[cfg(local_speech)]
type Loaded = Arc<Mutex<Option<egui_bench::speech::Transcriber>>>;
#[cfg(not(local_speech))]
type Loaded = ();

pub struct Voice {
    state: State,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    loaded: Loaded,
    pub status: String,
}

enum Event {
    Status(String),
    Text(String),
    Failed(String),
}

impl Default for Voice {
    fn default() -> Self {
        let (tx, rx) = channel();
        Voice { state: State::Idle, tx, rx, loaded: Default::default(), status: String::new() }
    }
}

impl Voice {
    pub fn listening(&self) -> bool {
        matches!(self.state, State::Listening(_))
    }

    pub fn reading(&self) -> bool {
        matches!(self.state, State::Reading)
    }

    pub fn busy(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    pub fn seconds(&self) -> f32 {
        match &self.state {
            State::Listening(t) => t.since.elapsed().as_secs_f32(),
            _ => 0.0,
        }
    }

    pub fn level(&self) -> f32 {
        match &self.state {
            State::Listening(t) => f32::from_bits(t.peak.swap(0, Ordering::Relaxed)),
            _ => 0.0,
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        if self.busy() {
            return Ok(());
        }
        if !crate::platform::microphone_allowed() {
            crate::platform::ask_for_microphone()?;
            return Err("allow the microphone in the box that just opened, then hold the button again".into());
        }
        self.state = State::Listening(Box::new(open()?));
        self.status.clear();
        Ok(())
    }

    pub fn cancel(&mut self) {
        if self.listening() {
            self.state = State::Idle;
        }
    }

    pub fn finish(&mut self, settings: &Settings, wake: impl Fn() + Send + Sync + 'static) {
        let State::Listening(take) = std::mem::replace(&mut self.state, State::Idle) else { return };
        let rate = take.rate;
        let heard = std::mem::take(&mut *take.heard.lock().unwrap_or_else(|e| e.into_inner()));
        drop(take);
        if let Err(why) = judge(&heard, rate) {
            let _ = self.tx.send(Event::Failed(why.into()));
            return;
        }
        if let Some(p) = settings.problem() {
            let _ = self.tx.send(Event::Failed(p.into()));
            return;
        }
        self.state = State::Reading;
        self.status = "listening".into();
        let pcm = super::to_whisper_rate(&heard, rate);
        self.read(pcm, settings, wake);
    }

    fn read(&self, pcm: Vec<f32>, settings: &Settings, wake: impl Fn() + Send + Sync + 'static) {
        let tx = self.tx.clone();
        let settings = settings.clone();
        let loaded = self.loaded.clone();
        std::thread::Builder::new()
            .name("colouring-speech".into())
            .spawn(move || {
                let said = if settings.here {
                    read_here(&settings, &pcm, &tx, &wake, &loaded)
                } else {
                    read_on_server(&settings, &pcm)
                };
                let _ = tx.send(match said {
                    Ok(t) => Event::Text(t),
                    Err(e) => Event::Failed(e),
                });
                wake();
            })
            .ok();
    }

    pub fn poll(&mut self) -> Vec<Said> {
        let mut out = Vec::new();
        for ev in self.rx.try_iter().collect::<Vec<_>>() {
            match ev {
                Event::Status(s) => self.status = s,
                Event::Text(t) => {
                    self.state = State::Idle;
                    self.status.clear();
                    out.push(Said::Words(super::clean_words(&t)));
                }
                Event::Failed(e) => {
                    self.state = State::Idle;
                    self.status.clear();
                    out.push(Said::Failed(e));
                }
            }
        }
        out
    }
}

#[cfg(local_speech)]
pub fn models_dir() -> std::path::PathBuf {
    let shared = dirs_shared();
    shared.unwrap_or_else(|| crate::settings::data_dir().join("models"))
}

#[cfg(local_speech)]
fn dirs_shared() -> Option<std::path::PathBuf> {
    let own = crate::settings::data_dir().join("models");
    let theirs = std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".local/share/agentbench/models"))?;
    let model = egui_bench::voice::DEFAULT_MODEL;
    (!own.join(model).exists() && theirs.join(model).join("model.safetensors").exists()).then_some(theirs)
}

#[cfg(local_speech)]
fn read_here(
    settings: &Settings,
    pcm: &[f32],
    tx: &Sender<Event>,
    wake: &(impl Fn() + Send + Sync + 'static),
    loaded: &Loaded,
) -> Result<String, String> {
    let mut slot = loaded.lock().unwrap_or_else(|e| e.into_inner());
    let t = slot.get_or_insert_with(|| egui_bench::speech::Transcriber::new(models_dir()));
    let id = if egui_bench::voice::model(&settings.model).is_some() {
        settings.model.as_str()
    } else {
        egui_bench::voice::DEFAULT_MODEL
    };
    if !t.installed(id) {
        t.download(id, &|p| {
            let _ = tx.send(Event::Status(format!("fetching the ears: {} MB", p.got_mb)));
            wake();
        })?;
    }
    let _ = tx.send(Event::Status("listening".into()));
    wake();
    t.load(id)?;
    t.transcribe(pcm)
}

#[cfg(not(local_speech))]
fn read_here(
    _: &Settings,
    _: &[f32],
    _: &Sender<Event>,
    _: &(impl Fn() + Send + Sync + 'static),
    _: &Loaded,
) -> Result<String, String> {
    Err("this build cannot listen on its own, set a speech server in settings".into())
}

pub fn read_on_server(settings: &Settings, pcm: &[f32]) -> Result<String, String> {
    egui_bench::voice::Server { url: settings.url.clone(), key: settings.key.clone() }
        .transcribe(pcm, &settings.server_model)
}

fn open() -> Result<Take, String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "no microphone found, plug one in or type the request".to_string())?;
    let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "microphone".into());
    let config = device.default_input_config().map_err(|e| format!("{name}: {e}"))?;
    let (rate, channels) = (config.sample_rate(), config.channels() as usize);
    let heard = Arc::new(Mutex::new(Vec::with_capacity(rate as usize * 10)));
    let peak = Arc::new(AtomicU32::new(0));
    let limit = rate as usize * MAX_SECONDS;
    let cfg = config.config();
    let err = |e: cpal::Error| eprintln!("colouring-pages: microphone error: {e}");

    macro_rules! build {
        ($t:ty) => {{
            let (heard, peak) = (heard.clone(), peak.clone());
            device.build_input_stream(
                cfg,
                move |data: &[$t], _| push(data, channels, limit, &heard, &peak),
                err,
                None,
            )
        }};
    }
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::U16 => build!(u16),
        cpal::SampleFormat::I32 => build!(i32),
        other => return Err(format!("{name}: sample format {other:?} is not supported")),
    }
    .map_err(|e| format!("{name}: {e}"))?;
    stream.play().map_err(|e| format!("{name}: {e}"))?;
    Ok(Take { _stream: stream, heard, peak, rate, since: Instant::now() })
}

fn push<T: cpal::SizedSample>(
    data: &[T],
    channels: usize,
    limit: usize,
    heard: &Mutex<Vec<f32>>,
    peak: &AtomicU32,
) where
    f32: cpal::FromSample<T>,
{
    let Ok(mut heard) = heard.lock() else { return };
    let mut loudest = 0.0f32;
    for frame in data.chunks(channels.max(1)) {
        let v = frame.iter().map(|s| cpal::Sample::to_sample::<f32>(*s)).sum::<f32>() / frame.len() as f32;
        loudest = loudest.max(v.abs());
        if heard.len() < limit {
            heard.push(v);
        }
    }
    let mut cur = peak.load(Ordering::Relaxed);
    while f32::from_bits(cur) < loudest {
        match peak.compare_exchange_weak(cur, loudest.to_bits(), Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(now) => cur = now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Reply, serve};

    #[test]
    fn a_take_is_sent_to_the_speech_server_as_a_wav_with_the_key() {
        let fake = serve(|_| Reply::json(serde_json::json!({"text": " cute christmas cats "})));
        let s = Settings {
            here: false,
            model: String::new(),
            url: format!("{}/v1", fake.url),
            key: "k".into(),
            server_model: "whisper-large".into(),
        };
        let got = read_on_server(&s, &vec![0.1; 16_000]).unwrap();
        assert_eq!(got, "cute christmas cats");
        let seen = &fake.requests()[0];
        assert_eq!(seen.path, "/v1/audio/transcriptions");
        assert!(seen.headers.contains("authorization: bearer k"));
        assert!(seen.text().contains("whisper-large") && seen.text().contains("RIFF"));
    }

    #[test]
    fn a_failing_speech_server_is_reported() {
        let fake = serve(|_| Reply::status(500, "boom"));
        let s = Settings { here: false, model: String::new(), url: fake.url.clone(), key: String::new(), server_model: String::new() };
        assert!(read_on_server(&s, &vec![0.1; 1600]).unwrap_err().contains("500"));
    }

    #[test]
    fn samples_are_mixed_to_mono_and_the_limit_holds() {
        let heard = Mutex::new(Vec::new());
        let peak = AtomicU32::new(0);
        push(&[0.5f32, -0.5, 1.0, 1.0, 0.2, 0.2], 2, 2, &heard, &peak);
        assert_eq!(*heard.lock().unwrap(), vec![0.0, 1.0]);
        assert_eq!(f32::from_bits(peak.load(Ordering::Relaxed)), 1.0);
    }
}
