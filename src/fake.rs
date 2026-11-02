use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub headers: String,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_default()
    }
}

pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(value: serde_json::Value) -> Reply {
        Reply { status: 200, content_type: "application/json", body: value.to_string().into_bytes() }
    }

    pub fn bytes(content_type: &'static str, body: Vec<u8>) -> Reply {
        Reply { status: 200, content_type, body }
    }

    pub fn status(status: u16, body: &str) -> Reply {
        Reply { status, content_type: "text/plain", body: body.as_bytes().to_vec() }
    }
}

pub struct Fake {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
}

impl Fake {
    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

pub fn serve(handler: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Fake {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(handler);
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let (handler, log) = (handler.clone(), log.clone());
            std::thread::spawn(move || answer(stream, &*handler, &log));
        }
    });
    Fake { url, seen }
}

fn answer(mut stream: std::net::TcpStream, handler: &dyn Fn(&Seen) -> Reply, log: &Mutex<Vec<Seen>>) {
    let mut got = Vec::new();
    let mut buf = [0u8; 16384];
    let (head_len, want) = loop {
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
            return;
        }
        got.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&got).to_string();
        if let Some((head, _)) = text.split_once("\r\n\r\n") {
            let want = head
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            break (head.len() + 4, want);
        }
    };
    while got.len() < head_len + want {
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    let head = String::from_utf8_lossy(&got[..head_len]).to_string();
    let mut first = head.lines().next().unwrap_or_default().split_whitespace();
    let seen = Seen {
        method: first.next().unwrap_or_default().to_string(),
        path: first.next().unwrap_or_default().to_string(),
        headers: head.to_lowercase(),
        body: got[head_len..].to_vec(),
    };
    log.lock().unwrap().push(seen.clone());
    let reply = handler(&seen);
    let out = format!(
        "HTTP/1.1 {} X\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        reply.status,
        reply.content_type,
        reply.body.len()
    );
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.write_all(&reply.body);
}
