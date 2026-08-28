//! Conformance against the Workspaces API contract.
//!
//! `fixtures/workspaces-annotations.json` is extracted from `api-design/spec.yaml` in the
//! Workspaces repo: every documented example for the annotation endpoints, plus the
//! resolved JSON Schemas. Two guarantees are checked:
//!
//! 1. every documented response example deserializes into our types and re-serializes to
//!    the same JSON (no field lost, no field invented);
//! 2. every request body we could send validates against the request schema.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "conformance tests assert by panicking with the example name"
)]

use jsonschema::Validator;
use plannotator_tui_schema::{
    Anchor, Annotation, ApiError, CreateAnnotation, Kind, PatchAnnotation, SourceRange,
};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/workspaces-annotations.json")).expect("fixture is valid JSON")
}

fn examples<'a>(fixture: &'a Value, group: &str) -> impl Iterator<Item = (&'a str, &'a Value)> {
    fixture["examples"][group].as_object().expect("example group").iter().map(|(k, v)| (k.as_str(), v))
}

fn validator(fixture: &Value, schema: &str) -> Validator {
    jsonschema::validator_for(&fixture["schemas"][schema]).expect("schema compiles")
}

#[test]
fn documented_annotation_responses_round_trip() {
    let fixture = fixture();
    for (name, example) in examples(&fixture, "create_response") {
        let parsed: Annotation =
            serde_json::from_value(example.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = serde_json::to_value(&parsed).expect("serializable");
        assert_eq!(&back, example, "{name}: re-serialized JSON differs");
    }
    for (name, example) in examples(&fixture, "list_response") {
        let items = example["annotations"].as_array().unwrap_or_else(|| panic!("{name}: annotations array"));
        for item in items {
            let parsed: Annotation =
                serde_json::from_value(item.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(serde_json::to_value(&parsed).expect("serializable"), *item, "{name}");
        }
    }
}

#[test]
fn documented_request_examples_parse_and_validate() {
    let fixture = fixture();
    let create = validator(&fixture, "CreateAnnotationRequest");
    for (name, example) in examples(&fixture, "create_request") {
        let parsed: CreateAnnotation =
            serde_json::from_value(example.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = serde_json::to_value(&parsed).expect("serializable");
        assert_eq!(&back, example, "{name}");
        assert!(create.is_valid(&back), "{name}: does not validate");
    }
    let patch = validator(&fixture, "PatchAnnotationRequest");
    for (name, example) in examples(&fixture, "patch_request") {
        let parsed: PatchAnnotation =
            serde_json::from_value(example.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = serde_json::to_value(&parsed).expect("serializable");
        assert_eq!(&back, example, "{name}");
        assert!(patch.is_valid(&back), "{name}: does not validate");
    }
}

#[test]
fn our_create_request_validates_against_the_contract() {
    let fixture = fixture();
    let create = validator(&fixture, "CreateAnnotationRequest");
    let source = "Ship the **login page** by Friday.";
    let range =
        SourceRange { start: 11, end: 21, version: "0123456789abcdef0123456789abcdef01234567".into() };
    let request = CreateAnnotation {
        anchor: Anchor::new("login page", source, range, Kind::LooksGood, Some(0)),
        body: String::new(),
        author: Some("ramos@plannotator-tui".into()),
        attachments: Vec::new(),
    };
    let json = serde_json::to_value(&request).expect("serializable");
    let errors: Vec<String> = create.iter_errors(&json).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    assert!(serde_json::to_vec(&json["anchor"]).expect("serializable").len() < 16 * 1024);
}

#[test]
fn error_envelope_parses() {
    let err: ApiError = serde_json::from_value(serde_json::json!({
        "error": {"code": "validation_error", "message": "anchor.point.x out of range",
                  "details": {"fields": {"anchor.point.x": "must be between 0 and 1"}}}
    }))
    .expect("parses");
    assert_eq!(err.to_string(), "validation_error: anchor.point.x out of range");
}
