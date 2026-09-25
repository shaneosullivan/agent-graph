//! Everything we emit must validate against the published schema.

mod common;

use agent_graph::adapter::Capture;
use agent_graph::emit::to_line;
use agent_graph::event::EVENT_TYPES;
use common::*;
use serde_json::Value;

fn validator() -> jsonschema::Validator {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/event.schema.json");
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    jsonschema::validator_for(&schema).expect("schema compiles")
}

#[test]
fn emitted_events_validate() {
    let validator = validator();
    let payloads = fixture("claude-code/session.jsonl");
    for capture in [Capture { bodies: false }, Capture { bodies: true }] {
        for event in translate(&payloads, capture) {
            // Check the line exactly as written to disk.
            let line: Value = serde_json::from_str(&to_line(&event)).unwrap();
            let errors: Vec<String> = validator
                .iter_errors(&line)
                .map(|e| e.to_string())
                .collect();
            assert!(errors.is_empty(), "{line}\n{errors:#?}");
        }
    }
}

#[test]
fn schema_lists_every_event_type() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/event.schema.json");
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let listed: Vec<&str> = schema["properties"]["type"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(listed, EVENT_TYPES);
}

#[test]
fn schema_rejects_malformed_events() {
    let validator = validator();
    let bad = serde_json::json!({
        "v": 1, "id": "not-a-ulid", "ts": "2026-09-25T10:00:00.000Z",
        "type": "status", "node": "claude-code:s", "data": {"state": "sleeping"}
    });
    assert!(!validator.is_valid(&bad));
}
