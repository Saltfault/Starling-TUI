use anyhow::Context;

pub trait Clipboard {
    fn set_text(&mut self, text: &str) -> anyhow::Result<()>;
}

pub struct SystemClipboard {
    inner: arboard::Clipboard,
}

impl SystemClipboard {
    pub fn new() -> anyhow::Result<Self> {
        arboard::Clipboard::new()
            .map(|inner| Self { inner })
            .context("system clipboard is unavailable")
    }
}

impl Clipboard for SystemClipboard {
    fn set_text(&mut self, text: &str) -> anyhow::Result<()> {
        self.inner
            .set_text(text.to_owned())
            .context("clipboard is unavailable")
    }
}

#[cfg(test)]
use std::sync::{Arc, Mutex};

#[cfg(test)]
use anyhow::anyhow;

#[cfg(test)]
#[derive(Clone, Default)]
pub struct FakeClipboard {
    state: Arc<Mutex<FakeState>>,
}

#[cfg(test)]
#[derive(Default)]
struct FakeState {
    text: Option<String>,
    unavailable: bool,
}

#[cfg(test)]
impl FakeClipboard {
    pub fn with_text(text: impl Into<String>) -> Self {
        let clipboard = Self::default();
        clipboard
            .state
            .lock()
            .expect("fake clipboard poisoned")
            .text = Some(text.into());
        clipboard
    }

    pub fn unavailable() -> Self {
        let clipboard = Self::default();
        clipboard
            .state
            .lock()
            .expect("fake clipboard poisoned")
            .unavailable = true;
        clipboard
    }

    pub fn last_text(&self) -> Option<String> {
        self.state.lock().ok().and_then(|state| state.text.clone())
    }
}

#[cfg(test)]
impl Clipboard for FakeClipboard {
    fn set_text(&mut self, text: &str) -> anyhow::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("fake clipboard poisoned"))?;
        if state.unavailable {
            return Err(anyhow!("clipboard is unavailable"));
        }
        state.text = Some(text.to_owned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Clipboard, FakeClipboard};

    #[test]
    fn fake_round_trips_text_and_shares_state() {
        let mut writer = FakeClipboard::default();
        writer.set_text("a starling").unwrap();
        assert_eq!(writer.last_text().as_deref(), Some("a starling"));
    }

    #[test]
    fn fake_reports_unavailable_cleanly() {
        let mut unavailable = FakeClipboard::unavailable();
        assert_eq!(
            unavailable.set_text("x").unwrap_err().to_string(),
            "clipboard is unavailable"
        );
        assert!(unavailable.last_text().is_none());
    }

    #[test]
    fn fake_can_start_with_text() {
        let clipboard = FakeClipboard::with_text("seed");
        assert_eq!(clipboard.last_text().as_deref(), Some("seed"));
    }
}
