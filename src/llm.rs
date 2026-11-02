use crate::net;
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Llm {
    pub url: String,
    pub key: String,
    pub model: String,
}

impl Llm {
    pub fn ready(&self) -> bool {
        !self.url.trim().is_empty() && !self.model.trim().is_empty()
    }

    pub fn endpoint(&self) -> String {
        let base = net::base_url(&self.url);
        if base.ends_with("/chat/completions") {
            base
        } else {
            format!("{base}/chat/completions")
        }
    }

    pub async fn chat(&self, system: &str, user: &str) -> Result<String, String> {
        let url = self.endpoint();
        let body = json!({
            "model": self.model.trim(),
            "temperature": 0.8,
            "max_tokens": 1500,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });
        let mut req = net::http().post(&url).json(&body);
        if !self.key.trim().is_empty() {
            req = req.bearer_auth(self.key.trim());
        }
        let resp = req
            .timeout(Duration::from_secs(90))
            .send()
            .await
            .map_err(|e| format!("could not reach the language model at {url}: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("{url}: {e}"))?;
        if !status.is_success() {
            return Err(format!("the language model said {status}: {}", clip(text.trim(), 300)));
        }
        reply_text(&text)
    }
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

pub fn reply_text(body: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(body)
        .map_err(|_| format!("the language model sent something unreadable: {}", clip(body, 120)))?;
    let message = v.pointer("/choices/0/message");
    let content = message.and_then(|m| m.get("content"));
    let text = match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    let text = strip_thinking(&text);
    if text.trim().is_empty() {
        if let Some(e) = v.pointer("/error/message").and_then(|e| e.as_str()) {
            return Err(format!("the language model said: {e}"));
        }
        return Err("the language model sent an empty answer".into());
    }
    Ok(text)
}

pub fn strip_thinking(text: &str) -> String {
    let mut out = text.to_string();
    while let Some(start) = out.find("<think>") {
        match out[start..].find("</think>") {
            Some(end) => out.replace_range(start..start + end + "</think>".len(), ""),
            None => {
                out.truncate(start);
                break;
            }
        }
    }
    if let Some(end) = out.find("</think>") {
        out.replace_range(..end + "</think>".len(), "");
    }
    out.trim().to_string()
}

pub fn json_object(text: &str) -> Option<Value> {
    let text = strip_thinking(text);
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    for (i, (start, c)) in bytes.iter().enumerate() {
        if *c != '{' && *c != '[' {
            continue;
        }
        let mut stream = serde_json::Deserializer::from_str(&text[*start..]).into_iter::<Value>();
        if let Some(Ok(v)) = stream.next() {
            if v.is_object() || v.is_array() {
                return Some(v);
            }
        }
        let _ = i;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Reply, serve};

    #[test]
    fn json_is_found_inside_fences_and_chatter() {
        let v = json_object("Sure! Here you go:\n```json\n{\"a\": 1}\n```\nEnjoy").unwrap();
        assert_eq!(v["a"], 1);
        let v = json_object("<think>{\"x\":0}</think>[\"one\", \"two\"]").unwrap();
        assert_eq!(v[1], "two");
        assert!(json_object("no json here, just { words").is_none());
        assert!(json_object("").is_none());
    }

    #[test]
    fn thinking_is_removed_even_when_unterminated() {
        assert_eq!(strip_thinking("<think>hmm</think>answer"), "answer");
        assert_eq!(strip_thinking("hmm</think>answer"), "answer");
        assert_eq!(strip_thinking("answer<think>never ends"), "answer");
    }

    #[test]
    fn reply_text_reads_string_and_part_content() {
        let a = r#"{"choices":[{"message":{"content":"hi"}}]}"#;
        assert_eq!(reply_text(a).unwrap(), "hi");
        let b = r#"{"choices":[{"message":{"content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}}]}"#;
        assert_eq!(reply_text(b).unwrap(), "ab");
        assert!(reply_text("<html>").unwrap_err().contains("unreadable"));
        assert!(reply_text(r#"{"choices":[]}"#).unwrap_err().contains("empty"));
        let e = r#"{"error":{"message":"quota"}}"#;
        assert!(reply_text(e).unwrap_err().contains("quota"));
    }

    #[test]
    fn the_endpoint_is_built_from_any_base() {
        let l = |u: &str| Llm { url: u.into(), key: String::new(), model: "m".into() };
        assert_eq!(l("https://yalr.v0l.io/v1/").endpoint(), "https://yalr.v0l.io/v1/chat/completions");
        assert_eq!(l("localhost:8001/v1").endpoint(), "http://localhost:8001/v1/chat/completions");
        assert_eq!(
            l("http://x/v1/chat/completions").endpoint(),
            "http://x/v1/chat/completions"
        );
    }

    #[test]
    fn a_chat_is_posted_with_the_key_and_model() {
        let fake = serve(|_| Reply::json(json!({"choices":[{"message":{"content":"ok"}}]})));
        let llm = Llm { url: format!("{}/v1", fake.url), key: "secret".into(), model: "fast".into() };
        let got = net::runtime().block_on(llm.chat("be brief", "hello")).unwrap();
        assert_eq!(got, "ok");
        let seen = &fake.requests()[0];
        assert_eq!(seen.path, "/v1/chat/completions");
        assert!(seen.headers.contains("authorization: bearer secret"));
        let body = seen.json();
        assert_eq!(body["model"], "fast");
        assert_eq!(body["messages"][0]["content"], "be brief");
        assert_eq!(body["messages"][1]["content"], "hello");
    }

    #[test]
    fn a_server_error_is_reported() {
        let fake = serve(|_| Reply::status(429, "slow down"));
        let llm = Llm { url: fake.url.clone(), key: String::new(), model: "m".into() };
        let e = net::runtime().block_on(llm.chat("s", "u")).unwrap_err();
        assert!(e.contains("429") && e.contains("slow down"), "{e}");
    }
}
