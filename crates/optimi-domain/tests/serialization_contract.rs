use optimi_domain::catalog::Catalog;
use optimi_domain::id::ActionId;
use optimi_domain::model::{Observation, ReadFailure};
use serde_json::json;

#[test]
fn unit_read_failure_carries_its_kind() {
    assert_eq!(
        serde_json::to_value(ReadFailure::AccessDenied).expect("read failure serializes"),
        json!({ "kind": "access_denied" })
    );
}

#[test]
fn system_error_read_failure_carries_its_kind_and_code() {
    assert_eq!(
        serde_json::to_value(ReadFailure::SystemError { code: 5 })
            .expect("read failure serializes"),
        json!({ "kind": "system_error", "code": 5 })
    );
}

#[test]
fn observation_surfaces_a_system_error_failure_with_its_code() {
    assert_eq!(
        serde_json::to_value(Observation::<u32>::unavailable(ReadFailure::SystemError {
            code: 5
        }))
        .expect("observation serializes"),
        json!({ "state": "unavailable", "failure": { "kind": "system_error", "code": 5 } })
    );
}

#[test]
fn evidence_serializes_a_claim_key_instead_of_claim_text() {
    let catalog = Catalog::embedded();
    let action = catalog
        .action(&ActionId::parse("diagtrack-privacy").expect("action id is valid"))
        .expect("embedded action exists");

    let value = serde_json::to_value(&action.evidence).expect("evidence serializes");

    assert!(value.get("claim").is_none());
    assert_eq!(value["claim_key"], "claim.diagtrack-privacy");
}
