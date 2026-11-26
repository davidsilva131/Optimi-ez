use optimi_domain::catalog::Catalog;
use optimi_domain::command::Refusal;
use optimi_domain::id::{ActionId, TransactionId};
use optimi_domain::model::{
    Gate, Observation, ReadFailure, ServiceStartType, ServiceState, VerifiedChange,
};
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

#[test]
fn verified_change_serializes_the_read_back_state() {
    let change = VerifiedChange {
        transaction_id: TransactionId::parse("tx-1").expect("transaction id is valid"),
        action_id: ActionId::parse("diagtrack-privacy").expect("action id is valid"),
        service: "DiagTrack".to_owned(),
        verified_state: ServiceState {
            run_state: optimi_domain::model::ServiceRunState::Stopped,
            start_type: ServiceStartType::Disabled,
            delayed_start: false,
        },
    };

    assert_eq!(
        serde_json::to_value(&change).expect("verified change serializes"),
        json!({
            "transaction_id": "tx-1",
            "action_id": "diagtrack-privacy",
            "service": "DiagTrack",
            "verified_state": {
                "run_state": "stopped",
                "start_type": "disabled",
                "delayed_start": false
            }
        })
    );
}

#[test]
fn a_gate_refusal_carries_the_gate() {
    assert_eq!(
        serde_json::to_value(Refusal::GateBlocked {
            gate: Gate::AntiCheat
        })
        .expect("refusal serializes"),
        json!({ "reason": "gate_blocked", "gate": "anti_cheat" })
    );
}
