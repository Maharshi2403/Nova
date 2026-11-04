use std::fs;
use std::path::{Path, PathBuf};

use harness_core::protocol::{Envelope, EnvelopeError};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures(kind: &str) -> Vec<(String, String)> {
    let dir = repo_root().join("tests/fixtures/protocol").join(kind);
    let mut out: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read_to_string(&p).unwrap(),
            )
        })
        .collect();
    out.sort();
    assert!(!out.is_empty());
    out
}

fn schema(name: &str) -> serde_json::Value {
    let text = fs::read_to_string(repo_root().join("schemas").join(name)).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn valid_fixtures_pass_schema_and_round_trip() {
    let validator = jsonschema::validator_for(&schema("worker-envelope.schema.json")).unwrap();
    for (name, text) in fixtures("valid") {
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(validator.is_valid(&value), "schema rejected {name}");
        let env = Envelope::from_json(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let reparsed: serde_json::Value = serde_json::to_value(&env).unwrap();
        assert_eq!(reparsed, value, "round trip changed {name}");
    }
}

#[test]
fn invalid_fixtures_rejected_by_schema_and_rust() {
    let validator = jsonschema::validator_for(&schema("worker-envelope.schema.json")).unwrap();
    for (name, text) in fixtures("invalid") {
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(!validator.is_valid(&value), "schema accepted {name}");
        assert!(Envelope::from_json(&text).is_err(), "rust accepted {name}");
    }
}

#[test]
fn schema_rejects_wrong_version() {
    let text =
        fs::read_to_string(repo_root().join("tests/fixtures/protocol/invalid/wrong_version.json"))
            .unwrap();
    assert!(matches!(
        Envelope::from_json(&text),
        Err(EnvelopeError::Malformed(_))
    ));
}

#[test]
fn schemas_are_valid_json_schema_documents() {
    for name in [
        "worker-envelope.schema.json",
        "task.schema.json",
        "plan.schema.json",
        "priority-update.schema.json",
    ] {
        jsonschema::meta::validate(&schema(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}
