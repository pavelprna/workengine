//! One JSON stream for Workengine-supervised subprocess output (F22).

use std::io::{self, Write};

use serde::Serialize;
use tracing::info;

pub const STREAM_SCHEMA_VERSION: u32 = 1;
pub const EVENT_SPAWNED: &str = "spawned";
pub const EVENT_EXITED: &str = "exited";
pub const EVENT_KILLED: &str = "killed";
pub const EVENT_CHILD_STDOUT: &str = "child_stdout";
pub const EVENT_CHILD_STDERR: &str = "child_stderr";

pub const STREAM_EVENTS: &[&str] = &[
    EVENT_SPAWNED,
    EVENT_EXITED,
    EVENT_KILLED,
    EVENT_CHILD_STDOUT,
    EVENT_CHILD_STDERR,
];

#[derive(Serialize)]
struct StreamRecord<'a> {
    schema_version: u32,
    work_id: &'a str,
    event: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<&'a str>,
}

pub fn record_line(work_id: &str, event: &str, payload: Option<&str>) -> String {
    let rec = StreamRecord {
        schema_version: STREAM_SCHEMA_VERSION,
        work_id,
        event,
        payload,
    };
    serde_json::to_string(&rec).expect("stream fields are strings and integers")
}

pub(crate) fn emit(work_id: &str, event: &str, payload: Option<&str>) {
    info!(
        schema_version = STREAM_SCHEMA_VERSION,
        work_id,
        event,
        payload = payload.unwrap_or(""),
        "worker stream"
    );
    let line = record_line(work_id, event, payload);
    let _ = writeln!(io::stderr(), "{line}");
}
