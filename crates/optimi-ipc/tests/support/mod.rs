#![allow(dead_code)]

use optimi_domain::command::{
    AbandonedTransaction, CommandOutcome, Consent, RecoveryStatus, Refusal, RestoreReport,
};
use optimi_domain::id::{ActionId, PreviewId, ProfileId, TransactionId};
use optimi_domain::model::{
    Evidence, MeasurementStatus, Observation, Preview, Risk, ServiceRunState, ServiceStartType,
    ServiceState, VerifiedChange, WindowsIdentity, WindowsProductType, WindowsVersion,
};
use optimi_ipc::{PROTOCOL_VERSION, Response, ResponseBody};

pub fn running() -> ServiceState {
    ServiceState {
        run_state: ServiceRunState::Running,
        start_type: ServiceStartType::Automatic,
        delayed_start: true,
    }
}

pub fn target() -> ServiceState {
    ServiceState {
        run_state: ServiceRunState::Stopped,
        start_type: ServiceStartType::Disabled,
        delayed_start: false,
    }
}

pub fn profile_id() -> ProfileId {
    ProfileId::parse("competitive").expect("profile id is valid")
}

pub fn preview(id: &str) -> Preview {
    Preview {
        id: PreviewId::parse(id).expect("preview id is valid"),
        profile_id: profile_id(),
        catalog_version: "0.1".to_owned(),
        platform: optimi_domain::model::PlatformSupport::Supported,
        windows_version: Observation::observed(WindowsVersion {
            major: 10,
            minor: 0,
            build: 22631,
            product_type: WindowsProductType::Workstation,
        }),
        interactive_identity: Observation::observed(WindowsIdentity {
            account: "player".to_owned(),
            domain: Some("TEST".to_owned()),
        }),
        active_power_plan: Observation::observed(optimi_domain::model::ActivePowerPlan {
            guid: "381b4222-f694-41f0-9685-ff5bb260df2e".to_owned(),
            name: Some("Balanced".to_owned()),
        }),
        actions: vec![optimi_domain::model::ActionPreview {
            action_id: ActionId::parse("diagtrack-privacy").expect("action id is valid"),
            service: "DiagTrack".to_owned(),
            risk: Risk::Low,
            evidence: Evidence {
                source: "Microsoft Learn".to_owned(),
                url: "https://learn.microsoft.com".to_owned(),
                date: "2025-09-11".to_owned(),
                measurement: MeasurementStatus::NotMeasured,
                claim_key: "claim.diagtrack-privacy".to_owned(),
            },
            target: target(),
            observed: Observation::observed(running()),
            disposition: optimi_domain::model::Disposition::Applicable,
            reason: optimi_domain::model::DispositionReason::TargetDiffers,
        }],
    }
}

pub fn verified_change() -> VerifiedChange {
    VerifiedChange {
        transaction_id: TransactionId::parse("tx-1").expect("transaction id is valid"),
        action_id: ActionId::parse("diagtrack-privacy").expect("action id is valid"),
        service: "DiagTrack".to_owned(),
        verified_state: target(),
    }
}

pub fn response_frame(body: ResponseBody, request_id: u64) -> Vec<u8> {
    let response = Response {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        body,
    };
    optimi_ipc::encode_response(&response).expect("response encodes")
}

pub fn raw_frame(payload: &str) -> Vec<u8> {
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload.as_bytes());
    frame
}

pub fn refused() -> CommandOutcome<VerifiedChange> {
    CommandOutcome::Refused(Refusal::ConsentMismatch)
}

pub fn recovery_status() -> RecoveryStatus {
    RecoveryStatus {
        pending_transactions: Vec::new(),
        writes_permitted: true,
        blocker: None,
    }
}

pub fn abandoned() -> AbandonedTransaction {
    AbandonedTransaction {
        transaction_id: TransactionId::parse("tx-1").expect("transaction id is valid"),
        action_id: ActionId::parse("diagtrack-privacy").expect("action id is valid"),
        service: "DiagTrack".to_owned(),
        observed: Observation::observed(running()),
    }
}

pub fn restore_report() -> RestoreReport {
    RestoreReport {
        restored: vec![verified_change()],
        blocked: Vec::new(),
    }
}

pub fn consent(preview_id: PreviewId) -> Consent {
    Consent { preview_id }
}
