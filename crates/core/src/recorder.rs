//! Ported from `src/recorder.js`. Appends normalized events as NDJSON; the
//! input recording stays immutable. JS validated the event shape at record
//! time (`validateEvent`); in Rust an `Event` cannot be constructed with a
//! missing field or an unknown kind in the first place, so that check is
//! enforced by the type system instead of at runtime here.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::domain::Event;

#[derive(Debug)]
pub struct NdjsonRecorder {
    file_path: PathBuf,
}

impl NdjsonRecorder {
    pub fn new(file_path: impl AsRef<Path>) -> io::Result<Self> {
        let file_path = file_path.as_ref().to_path_buf();
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self { file_path })
    }

    pub fn record(&self, event: &Event) -> io::Result<()> {
        let line = serde_json::to_string(event).expect("Event serialization cannot fail");
        let mut file = OpenOptions::new().create(true).append(true).open(&self.file_path)?;
        writeln!(file, "{line}")
    }
}
