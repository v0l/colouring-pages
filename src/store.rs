use crate::clean::Paper;
use crate::plan::Plan;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Status {
    #[default]
    Done,
    Running,
    Failed(String),
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub request: String,
    pub made: u64,
    pub paper: String,
    pub plan: Option<Plan>,
    pub prompts: Vec<String>,
    pub pages: usize,
    #[serde(default)]
    pub status: Status,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub meta: Meta,
    pub dir: PathBuf,
}

impl Entry {
    pub fn page_path(&self, index: usize) -> PathBuf {
        self.dir.join(format!("page-{}.png", index + 1))
    }

    pub fn paper(&self) -> Paper {
        Paper::from_id(&self.meta.paper)
    }
}

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
}

pub fn new_id(made: u64) -> String {
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{made}-{n:03}-{:04x}", crate::net::random_seed(made ^ n) & 0xFFFF)
}

impl Store {
    pub fn new(root: &Path) -> Store {
        Store { root: root.join("history") }
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn save_meta(&self, meta: &Meta) -> Result<(), String> {
        let dir = self.dir(&meta.id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("meta.json"), text).map_err(|e| e.to_string())
    }

    pub fn save_page(&self, id: &str, index: usize, png: &[u8]) -> Result<(), String> {
        let dir = self.dir(id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(format!("page-{}.png", index + 1)), png).map_err(|e| e.to_string())
    }

    pub fn list(&self) -> Vec<Entry> {
        let Ok(read) = std::fs::read_dir(&self.root) else { return Vec::new() };
        let mut out: Vec<Entry> = read
            .flatten()
            .filter_map(|e| {
                let text = std::fs::read_to_string(e.path().join("meta.json")).ok()?;
                let meta: Meta = serde_json::from_str(&text).ok()?;
                Some(Entry { meta, dir: e.path() })
            })
            .collect();
        out.sort_by(|a, b| b.meta.made.cmp(&a.meta.made).then(b.meta.id.cmp(&a.meta.id)));
        out
    }

    pub fn mark_interrupted(&self) {
        for mut entry in self.list() {
            if entry.meta.status == Status::Running {
                entry.meta.status = Status::Interrupted;
                let _ = self.save_meta(&entry.meta);
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<Entry> {
        self.list().into_iter().find(|e| e.meta.id == id)
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        if id.is_empty() || id.contains(['/', '\\', '.']) {
            return Err("that is not a saved set".into());
        }
        std::fs::remove_dir_all(self.dir(id)).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, made: u64) -> Meta {
        Meta {
            id: id.into(),
            request: "cats".into(),
            made,
            paper: "a4".into(),
            plan: None,
            prompts: vec!["p".into()],
            pages: 1,
            status: Status::Done,
        }
    }

    #[test]
    fn a_set_still_running_when_the_app_closed_comes_back_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.save_meta(&Meta { status: Status::Running, ..meta("live", 2) }).unwrap();
        store.save_meta(&meta("done", 1)).unwrap();
        store.mark_interrupted();
        assert_eq!(store.get("live").unwrap().meta.status, Status::Interrupted);
        assert_eq!(store.get("done").unwrap().meta.status, Status::Done);
    }

    #[test]
    fn sets_saved_before_there_was_a_status_read_as_done() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("history/old")).unwrap();
        let old = r#"{"id":"old","request":"cats","made":1,"paper":"a4","plan":null,"prompts":[],"pages":4}"#;
        std::fs::write(dir.path().join("history/old/meta.json"), old).unwrap();
        assert_eq!(Store::new(dir.path()).get("old").unwrap().meta.status, Status::Done);
    }

    #[test]
    fn saved_sets_come_back_newest_first_with_their_pages() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.save_meta(&meta("a", 100)).unwrap();
        store.save_meta(&meta("b", 200)).unwrap();
        store.save_page("b", 0, b"png").unwrap();
        let list = store.list();
        assert_eq!(list.iter().map(|e| e.meta.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(std::fs::read(list[0].page_path(0)).unwrap(), b"png");
    }

    #[test]
    fn a_folder_without_readable_meta_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.save_meta(&meta("ok", 1)).unwrap();
        std::fs::create_dir_all(dir.path().join("history/junk")).unwrap();
        std::fs::write(dir.path().join("history/junk/meta.json"), "{").unwrap();
        assert_eq!(store.list().len(), 1);
        assert!(Store::new(&dir.path().join("empty")).list().is_empty());
    }

    #[test]
    fn removing_deletes_the_set_and_refuses_paths() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.save_meta(&meta("a", 1)).unwrap();
        assert!(store.remove("../x").is_err());
        assert!(store.remove("").is_err());
        store.remove("a").unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn ids_differ_within_a_second() {
        assert_ne!(new_id(5), new_id(5));
    }
}
