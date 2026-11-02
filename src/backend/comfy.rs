use super::{ImageBackend, Job};
use crate::net;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Comfy {
    pub url: String,
    pub steps: u32,
    pub guidance: f32,
    pub patience: Duration,
    pub poll: Duration,
}

impl Comfy {
    pub fn new(url: &str) -> Comfy {
        Comfy {
            url: net::base_url(url),
            steps: 22,
            guidance: 3.5,
            patience: Duration::from_secs(600),
            poll: Duration::from_millis(800),
        }
    }

    pub fn workflow(&self, job: &Job) -> Value {
        json!({
            "1": {"class_type": "UNETLoader", "inputs": {"unet_name": "flux1-dev.safetensors", "weight_dtype": "fp8_e4m3fn"}},
            "2": {"class_type": "DualCLIPLoader", "inputs": {"clip_name1": "t5xxl_fp8_e4m3fn.safetensors", "clip_name2": "clip_l.safetensors", "type": "flux"}},
            "3": {"class_type": "VAELoader", "inputs": {"vae_name": "flux_ae.safetensors"}},
            "4": {"class_type": "CLIPTextEncode", "inputs": {"text": job.prompt, "clip": ["2", 0]}},
            "5": {"class_type": "FluxGuidance", "inputs": {"conditioning": ["4", 0], "guidance": self.guidance}},
            "6": {"class_type": "EmptySD3LatentImage", "inputs": {"width": job.width, "height": job.height, "batch_size": 1}},
            "7": {"class_type": "KSampler", "inputs": {"model": ["1", 0], "seed": job.seed, "steps": self.steps, "cfg": 1.0, "sampler_name": "euler", "scheduler": "simple", "positive": ["5", 0], "negative": ["5", 0], "latent_image": ["6", 0], "denoise": 1.0}},
            "8": {"class_type": "VAEDecode", "inputs": {"samples": ["7", 0], "vae": ["3", 0]}},
            "9": {"class_type": "SaveImage", "inputs": {"images": ["8", 0], "filename_prefix": "colouring"}},
        })
    }
}

fn client_id(seed: u64) -> String {
    format!("colouring-pages-{seed}")
}

pub fn output_image(history: &Value, id: &str) -> Option<(String, String, String)> {
    let entry = history.get(id)?;
    let images = entry.pointer("/outputs/9/images")?.as_array()?;
    let first = images.first()?;
    let field = |k: &str| first.get(k).and_then(|v| v.as_str()).map(str::to_string);
    Some((field("filename")?, field("subfolder").unwrap_or_default(), field("type").unwrap_or_else(|| "output".into())))
}

fn failure(history: &Value, id: &str) -> Option<String> {
    let status = history.get(id)?.get("status")?;
    if status.get("status_str").and_then(|s| s.as_str()) != Some("error") {
        return None;
    }
    let message = status
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|m| {
            m.iter().find_map(|e| {
                let e = e.as_array()?;
                (e.first()?.as_str()? == "execution_error")
                    .then(|| e.get(1)?.get("exception_message")?.as_str().map(str::to_string))?
            })
        })
        .unwrap_or_else(|| "it failed".into());
    Some(message)
}

#[async_trait]
impl ImageBackend for Comfy {
    fn name(&self) -> &'static str {
        "comfyui"
    }

    async fn generate(&self, job: &Job) -> Result<Vec<u8>, String> {
        let http = net::http();
        let queue = format!("{}/prompt", self.url);
        let resp = http
            .post(&queue)
            .json(&json!({ "prompt": self.workflow(job), "client_id": client_id(job.seed) }))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("could not reach ComfyUI at {}: {e}", self.url))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("ComfyUI refused the job: {status}: {}", text.chars().take(300).collect::<String>()));
        }
        let id = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("prompt_id").and_then(|p| p.as_str()).map(str::to_string))
            .ok_or_else(|| "ComfyUI did not say which job it queued".to_string())?;

        let started = std::time::Instant::now();
        let image = loop {
            if started.elapsed() > self.patience {
                return Err("ComfyUI took too long to draw the page".into());
            }
            tokio::time::sleep(self.poll).await;
            let resp = http
                .get(format!("{}/history/{id}", self.url))
                .timeout(Duration::from_secs(30))
                .send()
                .await
                .map_err(|e| format!("lost ComfyUI: {e}"))?;
            let history: Value = resp.json().await.unwrap_or_default();
            if let Some(why) = failure(&history, &id) {
                return Err(format!("ComfyUI could not draw the page: {why}"));
            }
            if let Some(found) = output_image(&history, &id) {
                break found;
            }
        };
        let resp = http
            .get(format!("{}/view", self.url))
            .query(&[("filename", image.0.as_str()), ("subfolder", image.1.as_str()), ("type", image.2.as_str())])
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| format!("could not fetch the page from ComfyUI: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("ComfyUI would not give the page back: {}", resp.status()));
        }
        resp.bytes().await.map(|b| b.to_vec()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Reply, serve};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn job() -> Job {
        Job { prompt: "a cat".into(), negative: "shading".into(), width: 960, height: 1280, seed: 7 }
    }

    fn quick(url: &str) -> Comfy {
        let mut c = Comfy::new(url);
        c.poll = Duration::from_millis(10);
        c.patience = Duration::from_secs(5);
        c
    }

    #[test]
    fn the_workflow_carries_prompt_size_and_seed() {
        let w = Comfy::new("x").workflow(&job());
        assert_eq!(w["4"]["inputs"]["text"], "a cat");
        assert_eq!(w["6"]["inputs"]["width"], 960);
        assert_eq!(w["6"]["inputs"]["height"], 1280);
        assert_eq!(w["7"]["inputs"]["seed"], 7);
        assert_eq!(w["1"]["inputs"]["unet_name"], "flux1-dev.safetensors");
        assert_eq!(w["9"]["class_type"], "SaveImage");
    }

    #[test]
    fn a_page_is_queued_polled_and_fetched() {
        let polls = AtomicUsize::new(0);
        let fake = serve(move |seen| match (seen.method.as_str(), seen.path.split('?').next().unwrap()) {
            ("POST", "/prompt") => Reply::json(json!({"prompt_id": "abc"})),
            ("GET", "/history/abc") => {
                if polls.fetch_add(1, Ordering::SeqCst) < 2 {
                    Reply::json(json!({}))
                } else {
                    Reply::json(json!({"abc": {"outputs": {"9": {"images": [{"filename": "kg_1.png", "subfolder": "", "type": "output"}]}}}}))
                }
            }
            ("GET", "/view") => Reply::bytes("image/png", b"PNGDATA".to_vec()),
            _ => Reply::status(404, "no"),
        });
        let got = net::runtime().block_on(quick(&fake.url).generate(&job())).unwrap();
        assert_eq!(got, b"PNGDATA");
        let seen = fake.requests();
        assert_eq!(seen[0].path, "/prompt");
        assert_eq!(seen[0].json()["prompt"]["4"]["inputs"]["text"], "a cat");
        assert!(seen.last().unwrap().path.contains("filename=kg_1.png"));
        assert!(seen.iter().filter(|s| s.path.starts_with("/history")).count() >= 3);
    }

    #[test]
    fn a_refused_job_is_reported() {
        let fake = serve(|_| Reply::status(400, "bad workflow"));
        let e = net::runtime().block_on(quick(&fake.url).generate(&job())).unwrap_err();
        assert!(e.contains("400") && e.contains("bad workflow"), "{e}");
    }

    #[test]
    fn a_failed_run_is_reported_with_its_reason() {
        let fake = serve(|seen| {
            if seen.method == "POST" {
                Reply::json(json!({"prompt_id": "abc"}))
            } else {
                Reply::json(json!({"abc": {"status": {"status_str": "error", "messages": [["execution_error", {"exception_message": "out of memory"}]]}}}))
            }
        });
        let e = net::runtime().block_on(quick(&fake.url).generate(&job())).unwrap_err();
        assert!(e.contains("out of memory"), "{e}");
    }

    #[test]
    fn a_job_that_never_finishes_times_out() {
        let fake = serve(|seen| {
            if seen.method == "POST" { Reply::json(json!({"prompt_id": "abc"})) } else { Reply::json(json!({})) }
        });
        let mut c = quick(&fake.url);
        c.patience = Duration::from_millis(200);
        let e = net::runtime().block_on(c.generate(&job())).unwrap_err();
        assert!(e.contains("too long"), "{e}");
    }

    #[test]
    fn an_unreachable_server_is_named() {
        let e = net::runtime().block_on(quick("http://127.0.0.1:1").generate(&job())).unwrap_err();
        assert!(e.contains("could not reach ComfyUI"), "{e}");
    }
}
