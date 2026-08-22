//! Ported from test/recorder.test.js. The JS suite had a second test,
//! "rejects an event that fails schema validation" — that case has no Rust
//! equivalent: an incomplete or unknown-kind `Event` cannot be constructed
//! at all, so there is nothing left to reject at record() time.
mod support;

use std::fs;

use momentum_core::recorder::NdjsonRecorder;
use support::confirmed_candidate_events;

#[test]
fn ndjson_recorder_appends_validated_events_as_newline_delimited_json_creating_parent_directories() {
    let dir = std::env::temp_dir().join(format!("momentum-recorder-test-{}", std::process::id()));
    let file_path = dir.join("nested").join("events.ndjson");
    let _ = fs::remove_dir_all(&dir);

    let recorder = NdjsonRecorder::new(&file_path).expect("recorder should create parent directories");
    let events = confirmed_candidate_events();
    recorder.record(&events[0]).unwrap();
    recorder.record(&events[1]).unwrap();

    let contents = fs::read_to_string(&file_path).unwrap();
    let lines: Vec<&str> = contents.trim().lines().collect();
    assert_eq!(lines.len(), 2);

    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["id"], "token");
    assert_eq!(first["payload"]["kind"], "tokenCreated");

    let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(second["id"], "pool");

    fs::remove_dir_all(&dir).unwrap();
}
