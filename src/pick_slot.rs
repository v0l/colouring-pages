use std::path::PathBuf;
use std::sync::Mutex;

type Done = Box<dyn FnOnce(Option<PathBuf>) + Send>;

pub struct PickSlot(Mutex<Option<Done>>);

impl PickSlot {
    pub const fn new() -> Self {
        PickSlot(Mutex::new(None))
    }

    pub fn set(&self, done: impl FnOnce(Option<PathBuf>) + Send + 'static) {
        let replaced = self.0.lock().unwrap_or_else(|e| e.into_inner()).replace(Box::new(done));
        if let Some(old) = replaced {
            old(None);
        }
    }

    pub fn finish(&self, picked: Option<PathBuf>) {
        let done = self.0.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(done) = done {
            done(picked);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_finished_pick_reaches_the_waiting_callback_once() {
        let slot = PickSlot::new();
        let (tx, rx) = mpsc::channel();
        slot.set(move |p| tx.send(p).unwrap());
        slot.finish(Some(PathBuf::from("/tmp/a.png")));
        slot.finish(Some(PathBuf::from("/tmp/b.png")));
        assert_eq!(rx.try_recv().unwrap(), Some(PathBuf::from("/tmp/a.png")));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_second_pick_cancels_the_first() {
        let slot = PickSlot::new();
        let (tx, rx) = mpsc::channel();
        let first = tx.clone();
        slot.set(move |p| first.send(("first", p)).unwrap());
        slot.set(move |p| tx.send(("second", p)).unwrap());
        slot.finish(Some(PathBuf::from("/x")));
        assert_eq!(rx.try_recv().unwrap(), ("first", None));
        assert_eq!(rx.try_recv().unwrap(), ("second", Some(PathBuf::from("/x"))));
    }
}
