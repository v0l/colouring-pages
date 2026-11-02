mod mic;

pub use mic::{Said, Voice};

pub use egui_bench::voice::audio::{to_whisper_rate, wav_bytes};

pub const MIN_SECONDS: f32 = 0.4;
pub const MIN_PEAK: f32 = 0.005;
pub const MAX_SECONDS: usize = 30;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub here: bool,
    pub model: String,
    pub url: String,
    pub key: String,
    pub server_model: String,
}

impl Settings {
    pub fn problem(&self) -> Option<&'static str> {
        if self.here {
            if cfg!(local_speech) {
                None
            } else {
                Some("this build cannot listen on its own, set a speech server in settings")
            }
        } else if self.url.trim().is_empty() {
            Some("set the speech server address in settings, or type the request")
        } else {
            None
        }
    }
}

pub fn judge(pcm: &[f32], rate: u32) -> Result<(), &'static str> {
    let seconds = pcm.len() as f32 / rate.max(1) as f32;
    if seconds < MIN_SECONDS {
        return Err("hold the button a little longer");
    }
    let peak = pcm.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak < MIN_PEAK {
        return Err("the microphone heard nothing, check it is not muted");
    }
    Ok(())
}

pub fn clean_words(text: &str) -> String {
    let t = text.trim();
    let t = t.trim_matches(|c: char| c == '[' || c == ']' || c == '(' || c == ')').trim();
    let lower = t.to_lowercase();
    let junk = ["blank_audio", "silence", "music", "thank you.", "thanks for watching."];
    if junk.iter().any(|j| lower == *j) {
        return String::new();
    }
    t.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_too_short_or_silent_take_is_turned_down_with_a_reason() {
        assert!(judge(&vec![0.5; 100], 16_000).unwrap_err().contains("longer"));
        assert!(judge(&vec![0.0; 16_000], 16_000).unwrap_err().contains("heard nothing"));
        assert!(judge(&vec![0.3; 16_000], 16_000).is_ok());
    }

    #[test]
    fn whisper_filler_is_not_a_request() {
        assert_eq!(clean_words(" [BLANK_AUDIO] "), "");
        assert_eq!(clean_words("(music)"), "");
        assert_eq!(clean_words(" cute christmas cats "), "cute christmas cats");
    }

    #[test]
    fn a_server_needs_an_address() {
        let mut s = Settings { here: false, model: String::new(), url: String::new(), key: String::new(), server_model: String::new() };
        assert!(s.problem().is_some());
        s.url = "http://x/v1".into();
        assert!(s.problem().is_none());
    }
}
