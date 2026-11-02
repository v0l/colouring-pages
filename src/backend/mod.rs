mod comfy;
mod openrouter;

pub use comfy::Comfy;
pub use openrouter::OpenRouter;

pub fn openrouter_default_model() -> &'static str {
    openrouter::DEFAULT_MODEL
}

use async_trait::async_trait;

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub prompt: String,
    pub negative: String,
    pub width: u32,
    pub height: u32,
    pub seed: u64,
}

#[async_trait]
pub trait ImageBackend: Send + Sync {
    fn name(&self) -> &'static str;

    async fn generate(&self, job: &Job) -> Result<Vec<u8>, String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Comfy,
    OpenRouter,
}

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Comfy => "comfy",
            Kind::OpenRouter => "openrouter",
        }
    }

    pub fn from_id(id: &str) -> Kind {
        match id {
            "openrouter" => Kind::OpenRouter,
            _ => Kind::Comfy,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Comfy => "comfyui flux on spark",
            Kind::OpenRouter => "openrouter",
        }
    }
}

pub fn generation_size(aspect: f32) -> (u32, u32) {
    let height = 1280u32;
    let width = ((height as f32 * aspect) / 16.0).round() as u32 * 16;
    (width.max(512), height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_multiples_of_sixteen() {
        for aspect in [0.707, 0.773, 0.75] {
            let (w, h) = generation_size(aspect);
            assert_eq!(w % 16, 0);
            assert_eq!(h % 16, 0);
            assert!((w as f32 / h as f32 - aspect).abs() < 0.02);
        }
    }

    #[test]
    fn kinds_round_trip_and_unknown_means_comfy() {
        for k in [Kind::Comfy, Kind::OpenRouter] {
            assert_eq!(Kind::from_id(k.id()), k);
        }
        assert_eq!(Kind::from_id("???"), Kind::Comfy);
    }
}
