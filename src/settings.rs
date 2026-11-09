use crate::backend::{Comfy, ImageBackend, Kind, OpenRouter};
use crate::clean::Paper;
use crate::detail::{Detail, Look};
use crate::llm::Llm;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_LLM_URL: &str = "https://yalr.v0l.io/v1";
pub const DEFAULT_LLM_MODEL: &str = "code";
pub const DEFAULT_VISION_MODEL: &str = "";
pub const DEFAULT_COMFY_URL: &str = "http://10.100.2.32:8188";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Listen {
    #[default]
    Here,
    Server,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default = "Settings::builtin")]
pub struct Settings {
    pub backend: String,
    pub comfy_url: String,
    pub openrouter_url: String,
    pub openrouter_key: String,
    pub openrouter_model: String,
    pub llm_url: String,
    pub llm_key: String,
    pub llm_model: String,
    pub vision_model: String,
    pub detail: Detail,
    pub close: bool,
    pub paper: String,
    pub listen: Listen,
    pub whisper_model: String,
    pub speech_url: String,
    pub speech_key: String,
    pub speech_model: String,
}

const BAKED: &str = include_str!(concat!(env!("OUT_DIR"), "/defaults.json"));

impl Default for Settings {
    fn default() -> Self {
        if cfg!(test) { Settings::builtin() } else { Settings::baked() }
    }
}

impl Settings {
    fn baked() -> Self {
        serde_json::from_str(BAKED).unwrap_or_else(|_| Settings::builtin())
    }

    fn builtin() -> Self {
        Settings {
            backend: Kind::Comfy.id().into(),
            comfy_url: DEFAULT_COMFY_URL.into(),
            openrouter_url: String::new(),
            openrouter_key: String::new(),
            openrouter_model: String::new(),
            llm_url: DEFAULT_LLM_URL.into(),
            llm_key: String::new(),
            llm_model: DEFAULT_LLM_MODEL.into(),
            vision_model: DEFAULT_VISION_MODEL.into(),
            detail: Detail::default(),
            close: false,
            paper: Paper::A4.id().into(),
            listen: if cfg!(target_os = "android") { Listen::Server } else { Listen::Here },
            whisper_model: egui_bench::voice::DEFAULT_MODEL.into(),
            speech_url: String::new(),
            speech_key: String::new(),
            speech_model: String::new(),
        }
    }

    pub fn kind(&self) -> Kind {
        Kind::from_id(&self.backend)
    }

    pub fn paper(&self) -> Paper {
        Paper::from_id(&self.paper)
    }

    pub fn llm(&self) -> Llm {
        Llm { url: self.llm_url.clone(), key: self.llm_key.clone(), model: self.llm_model.clone(), vision_model: self.vision_model.clone() }
    }

    pub fn look(&self) -> Look {
        Look { detail: self.detail, close: self.close }
    }

    pub fn image_backend(&self) -> Box<dyn ImageBackend> {
        match self.kind() {
            Kind::Comfy => Box::new(Comfy::new(&self.comfy_url)),
            Kind::OpenRouter => {
                Box::new(OpenRouter::new(&self.openrouter_url, &self.openrouter_key, &self.openrouter_model))
            }
        }
    }

    pub fn problem(&self) -> Option<&'static str> {
        if !self.llm().ready() {
            return Some("set the language model address and name in settings");
        }
        match self.kind() {
            Kind::Comfy if self.comfy_url.trim().is_empty() => Some("set the ComfyUI address in settings"),
            Kind::OpenRouter if self.openrouter_key.trim().is_empty() => {
                Some("add the OpenRouter key in settings")
            }
            _ => None,
        }
    }

    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

static DATA: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA.set(dir);
}

pub fn data_dir() -> PathBuf {
    DATA.get_or_init(default_data_dir).clone()
}

#[cfg(target_os = "ios")]
fn default_data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let dir = home.join("Library/Application Support/colouring-pages");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(all(not(target_os = "android"), not(target_os = "ios")))]
fn default_data_dir() -> PathBuf {
    std::env::var_os("COLOURING_PAGES_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|d| d.join("colouring-pages")))
        .unwrap_or_else(|| PathBuf::from("colouring-pages-data"))
}

#[cfg(target_os = "android")]
fn default_data_dir() -> PathBuf {
    PathBuf::from("/data/local/tmp/colouring-pages")
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep/settings.json");
        let mut s = Settings::default();
        s.openrouter_key = "sk-or".into();
        s.paper = "letter".into();
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        assert_eq!(s.paper(), Paper::Letter);
    }

    #[test]
    fn a_missing_or_damaged_file_gives_defaults_and_old_files_gain_new_fields() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(&dir.path().join("none.json")), Settings::default());
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "{ nope").unwrap();
        assert_eq!(Settings::load(&bad), Settings::default());
        let old = dir.path().join("old.json");
        std::fs::write(&old, r#"{"backend":"openrouter"}"#).unwrap();
        let s = Settings::load(&old);
        assert_eq!(s.kind(), Kind::OpenRouter);
        assert_eq!(s.llm_url, DEFAULT_LLM_URL);
    }

    #[test]
    fn problems_name_the_missing_piece() {
        let mut s = Settings::default();
        assert!(s.problem().is_none());
        s.backend = "openrouter".into();
        assert!(s.problem().unwrap().contains("OpenRouter"));
        s.openrouter_key = "k".into();
        assert!(s.problem().is_none());
        s.llm_model.clear();
        assert!(s.problem().unwrap().contains("language model"));
    }

    #[test]
    fn baked_defaults_fill_only_the_fields_they_name() {
        let s: Settings = serde_json::from_str(r#"{"paper":"letter"}"#).unwrap();
        assert_eq!(s.llm_url, DEFAULT_LLM_URL);
        assert_eq!(Settings::default(), Settings::builtin());
        assert!(serde_json::from_str::<Settings>(BAKED).is_ok());
    }

    #[test]
    fn the_detail_level_and_closeness_are_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut s = Settings::default();
        s.detail = Detail::VeryDetailed;
        s.close = true;
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert_eq!(back.look(), Look { detail: Detail::VeryDetailed, close: true });
        assert_eq!(Settings::default().look().detail, Detail::Medium);
    }

    #[test]
    fn the_backend_follows_the_setting() {
        let mut s = Settings::default();
        assert_eq!(s.image_backend().name(), "comfyui");
        s.backend = "openrouter".into();
        assert_eq!(s.image_backend().name(), "openrouter");
    }
}
