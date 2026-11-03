use super::{ImageBackend, Job};
use crate::{net, style};
use async_trait::async_trait;
use base64::Engine;
use serde_json::{Value, json};
use std::time::Duration;

pub const DEFAULT_MODEL: &str = "google/gemini-2.5-flash-image";
pub const DEFAULT_URL: &str = "https://openrouter.ai/api/v1";

#[derive(Clone, Debug)]
pub struct OpenRouter {
    pub url: String,
    pub key: String,
    pub model: String,
}

impl OpenRouter {
    pub fn new(url: &str, key: &str, model: &str) -> OpenRouter {
        let url = if url.trim().is_empty() { DEFAULT_URL } else { url };
        let model = if model.trim().is_empty() { DEFAULT_MODEL } else { model };
        OpenRouter { url: net::base_url(url), key: key.trim().to_string(), model: model.trim().to_string() }
    }

    pub fn request(&self, job: &Job) -> Value {
        let aspect = if job.width <= job.height { "3:4" } else { "4:3" };
        let words = format!("Draw this image: {}", style::with_negative_inline(&job.prompt));
        let content = match &job.reference {
            Some(png) => json!([
                {"type": "text", "text": format!("{words}\nUse the attached picture as a reference for what the main subject looks like, but redraw it as a clean black and white colouring page.")},
                {"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", mime(png), base64::engine::general_purpose::STANDARD.encode(png.as_slice()))}},
            ]),
            None => json!(words),
        };
        json!({
            "model": self.model,
            "modalities": ["image", "text"],
            "image_config": { "aspect_ratio": aspect },
            "messages": [{ "role": "user", "content": content }],
        })
    }
}

fn mime(bytes: &[u8]) -> &'static str {
    match bytes {
        [0xFF, 0xD8, ..] => "image/jpeg",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => "image/png",
    }
}

pub fn first_image(body: &str) -> Result<Vec<u8>, String> {
    let v: Value = serde_json::from_str(body)
        .map_err(|_| format!("OpenRouter sent something unreadable: {}", body.chars().take(120).collect::<String>()))?;
    if let Some(e) = v.pointer("/error/message").and_then(|e| e.as_str()) {
        return Err(format!("OpenRouter said: {e}"));
    }
    let message = v.pointer("/choices/0/message").ok_or("OpenRouter sent no answer")?;
    let url = message
        .get("images")
        .and_then(|i| i.as_array())
        .and_then(|i| i.first())
        .and_then(|i| i.pointer("/image_url/url").or_else(|| i.get("url")))
        .and_then(|u| u.as_str())
        .or_else(|| {
            message.get("content").and_then(|c| c.as_array()).and_then(|parts| {
                parts.iter().find_map(|p| p.pointer("/image_url/url").and_then(|u| u.as_str()))
            })
        })
        .ok_or("OpenRouter answered without a picture, try again")?;
    let (_, data) = url.split_once("base64,").ok_or("OpenRouter's picture was not inline")?;
    base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|e| format!("OpenRouter's picture was damaged: {e}"))
}

#[async_trait]
impl ImageBackend for OpenRouter {
    fn name(&self) -> &'static str {
        "openrouter"
    }

    async fn generate(&self, job: &Job) -> Result<Vec<u8>, String> {
        if self.key.is_empty() {
            return Err("add the OpenRouter key in settings".into());
        }
        let url = format!("{}/chat/completions", self.url);
        let resp = net::http()
            .post(&url)
            .bearer_auth(&self.key)
            .json(&self.request(job))
            .timeout(Duration::from_secs(180))
            .send()
            .await
            .map_err(|e| format!("could not reach OpenRouter: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("OpenRouter said {status}: {}", text.chars().take(300).collect::<String>()));
        }
        first_image(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Reply, serve};

    fn job() -> Job {
        Job { prompt: style::finish("a cat"), negative: style::NEGATIVE.into(), width: 960, height: 1280, seed: 1, reference: None }
    }

    fn data_url(bytes: &[u8]) -> String {
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    #[test]
    fn the_request_asks_for_an_image_with_the_style_and_negative() {
        let r = OpenRouter::new("", "k", "").request(&job());
        assert_eq!(r["modalities"][0], "image");
        assert_eq!(r["model"], DEFAULT_MODEL);
        let text = r["messages"][0]["content"].as_str().unwrap();
        assert!(text.contains(style::STYLE_SUFFIX) && text.contains("Do not include"));
    }

    #[test]
    fn a_reference_picture_is_sent_inline_beside_the_words() {
        let jpeg = vec![0xFF, 0xD8, 1, 2, 3];
        let job = Job { reference: Some(std::sync::Arc::new(jpeg.clone())), ..job() };
        let r = OpenRouter::new("", "k", "").request(&job);
        let parts = r["messages"][0]["content"].as_array().unwrap();
        assert!(parts[0]["text"].as_str().unwrap().contains("reference"));
        let url = parts[1]["image_url"]["url"].as_str().unwrap();
        let (head, data) = url.split_once("base64,").unwrap();
        assert_eq!(head, "data:image/jpeg;");
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(data).unwrap(), jpeg);
    }

    #[test]
    fn an_image_is_read_from_the_message() {
        let body = json!({"choices":[{"message":{"content":"","images":[{"type":"image_url","image_url":{"url": data_url(b"PNG!")}}]}}]});
        assert_eq!(first_image(&body.to_string()).unwrap(), b"PNG!");
    }

    #[test]
    fn an_image_in_content_parts_is_read_too() {
        let body = json!({"choices":[{"message":{"content":[{"type":"image_url","image_url":{"url": data_url(b"X")}}]}}]});
        assert_eq!(first_image(&body.to_string()).unwrap(), b"X");
    }

    #[test]
    fn bad_answers_are_errors() {
        assert!(first_image("<html>").unwrap_err().contains("unreadable"));
        assert!(first_image(r#"{"error":{"message":"no credits"}}"#).unwrap_err().contains("no credits"));
        assert!(first_image(r#"{"choices":[{"message":{"content":"sorry"}}]}"#).unwrap_err().contains("without a picture"));
        assert!(first_image(r#"{"choices":[{"message":{"images":[{"image_url":{"url":"http://x/y.png"}}]}}]}"#).unwrap_err().contains("not inline"));
        assert!(first_image(r#"{"choices":[{"message":{"images":[{"image_url":{"url":"data:image/png;base64,@@@"}}]}}]}"#).unwrap_err().contains("damaged"));
        assert!(first_image("{}").is_err());
    }

    #[test]
    fn a_page_is_requested_with_the_key_and_decoded() {
        let png = data_url(b"REALPNG");
        let fake = serve(move |_| Reply::json(json!({"choices":[{"message":{"images":[{"image_url":{"url": png}}]}}]})));
        let b = OpenRouter::new(&fake.url, "sk-or-1", "some/model");
        let got = net::runtime().block_on(b.generate(&job())).unwrap();
        assert_eq!(got, b"REALPNG");
        let seen = &fake.requests()[0];
        assert_eq!(seen.path, "/chat/completions");
        assert!(seen.headers.contains("authorization: bearer sk-or-1"));
        assert_eq!(seen.json()["model"], "some/model");
    }

    #[test]
    fn a_missing_key_is_said_before_any_request() {
        let fake = serve(|_| Reply::status(500, "no"));
        let b = OpenRouter::new(&fake.url, "", "");
        let e = net::runtime().block_on(b.generate(&job())).unwrap_err();
        assert!(e.contains("key"));
        assert!(fake.requests().is_empty());
    }

    #[test]
    fn an_http_error_is_reported() {
        let fake = serve(|_| Reply::status(402, "out of credits"));
        let b = OpenRouter::new(&fake.url, "k", "");
        let e = net::runtime().block_on(b.generate(&job())).unwrap_err();
        assert!(e.contains("402") && e.contains("out of credits"));
    }
}
