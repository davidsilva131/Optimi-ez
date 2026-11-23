use std::collections::HashMap;

use optimi_domain::adapter::SystemAdapter;
use optimi_domain::command::{
    CommandOutcome, Consent, DomainService, RecoveryStatus, Refusal, WriteBlocker, WriteCommand,
};
use optimi_domain::id::{PreviewId, ProfileId, TransactionId};
use optimi_domain::model::{
    ActivePowerPlan, Disposition, DispositionReason, Observation, PlatformSupport, ReadFailure,
    ServiceRunState, ServiceStartType, ServiceState, WindowsIdentity, WindowsVersion,
};

struct FakeSystemAdapter {
    version: Result<WindowsVersion, ReadFailure>,
    identity: Result<WindowsIdentity, ReadFailure>,
    power_plan: Result<ActivePowerPlan, ReadFailure>,
    services: HashMap<String, Result<ServiceState, ReadFailure>>,
}

impl FakeSystemAdapter {
    fn new() -> Self {
        Self {
            version: Ok(WindowsVersion {
                major: 10,
                minor: 0,
                build: 22631,
            }),
            identity: Ok(WindowsIdentity {
                account: "player".to_owned(),
                domain: Some("TEST".to_owned()),
            }),
            power_plan: Ok(ActivePowerPlan {
                guid: "381b4222-f694-41f0-9685-ff5bb260df2e".to_owned(),
                name: Some("Balanced".to_owned()),
            }),
            services: HashMap::new(),
        }
    }

    fn with_service(mut self, name: &str, state: ServiceState) -> Self {
        self.services.insert(name.to_owned(), Ok(state));
        self
    }

    fn with_service_failure(mut self, name: &str, failure: ReadFailure) -> Self {
        self.services.insert(name.to_owned(), Err(failure));
        self
    }

    fn with_version(mut self, version: WindowsVersion) -> Self {
        self.version = Ok(version);
        self
    }

    fn with_version_failure(mut self, failure: ReadFailure) -> Self {
        self.version = Err(failure);
        self
    }

    fn with_identity_failure(mut self, failure: ReadFailure) -> Self {
        self.identity = Err(failure);
        self
    }

    fn with_power_plan_failure(mut self, failure: ReadFailure) -> Self {
        self.power_plan = Err(failure);
        self
    }
}

impl SystemAdapter for FakeSystemAdapter {
    fn windows_version(&self) -> Result<WindowsVersion, ReadFailure> {
        self.version.clone()
    }

    fn interactive_identity(&self) -> Result<WindowsIdentity, ReadFailure> {
        self.identity.clone()
    }

    fn service_state(&self, service: &str) -> Result<ServiceState, ReadFailure> {
        self.services
            .get(service)
            .cloned()
            .unwrap_or(Err(ReadFailure::NotFound))
    }

    fn active_power_plan(&self) -> Result<ActivePowerPlan, ReadFailure> {
        self.power_plan.clone()
    }
}

fn service_state(run_state: ServiceRunState, start_type: ServiceStartType) -> ServiceState {
    ServiceState {
        run_state,
        start_type,
    }
}

fn profile() -> ProfileId {
    ProfileId::parse("competitive").expect("profile id is valid")
}

fn completed_preview(
    outcome: CommandOutcome<optimi_domain::model::Preview>,
) -> optimi_domain::model::Preview {
    let CommandOutcome::Completed(preview) = outcome else {
        panic!("expected a completed preview");
    };
    preview
}

#[test]
fn preview_reports_observed_service_state() {
    let domain = DomainService::new(FakeSystemAdapter::new().with_service(
        "DiagTrack",
        service_state(ServiceRunState::Running, ServiceStartType::Automatic),
    ));

    let preview = completed_preview(domain.preview(&profile()));

    assert_eq!(preview.platform, PlatformSupport::Supported);
    assert_eq!(preview.actions.len(), 1);
    assert_eq!(preview.actions[0].action_id.as_str(), "diagtrack-privacy");
    assert_eq!(preview.actions[0].disposition, Disposition::Applicable);
    assert_eq!(preview.actions[0].reason, DispositionReason::TargetDiffers);
    assert!(matches!(
        &preview.actions[0].observed,
        Observation::Observed { value } if value.run_state == ServiceRunState::Running
            && value.start_type == ServiceStartType::Automatic
    ));
    assert!(matches!(
        &preview.windows_version,
        Observation::Observed { value } if value.build == 22631
    ));
    assert!(matches!(
        &preview.interactive_identity,
        Observation::Observed { value } if value.account == "player" && value.domain.as_deref() == Some("TEST")
    ));
    assert!(matches!(
        &preview.active_power_plan,
        Observation::Observed { value } if value.name.as_deref() == Some("Balanced")
    ));
}

#[test]
fn preview_marks_already_disabled_service_as_no_op() {
    let domain = DomainService::new(FakeSystemAdapter::new().with_service(
        "DiagTrack",
        service_state(ServiceRunState::Stopped, ServiceStartType::Disabled),
    ));

    let preview = completed_preview(domain.preview(&profile()));

    assert_eq!(preview.actions[0].disposition, Disposition::NoOp);
    assert_eq!(
        preview.actions[0].reason,
        DispositionReason::AlreadyAtTarget
    );
}

#[test]
fn preview_surfaces_service_read_failure_as_unknown() {
    let domain = DomainService::new(
        FakeSystemAdapter::new().with_service_failure("DiagTrack", ReadFailure::AccessDenied),
    );

    let preview = completed_preview(domain.preview(&profile()));

    assert_eq!(preview.actions[0].disposition, Disposition::Unknown);
    assert_eq!(
        preview.actions[0].reason,
        DispositionReason::StateUnavailable
    );
    assert!(matches!(
        &preview.actions[0].observed,
        Observation::Unavailable {
            failure: ReadFailure::AccessDenied
        }
    ));
}

#[test]
fn preview_blocks_unsupported_windows_build() {
    let domain = DomainService::new(
        FakeSystemAdapter::new()
            .with_version(WindowsVersion {
                major: 10,
                minor: 0,
                build: 19045,
            })
            .with_service(
                "DiagTrack",
                service_state(ServiceRunState::Running, ServiceStartType::Automatic),
            ),
    );

    let preview = completed_preview(domain.preview(&profile()));

    assert_eq!(preview.platform, PlatformSupport::Unsupported);
    assert_eq!(preview.actions[0].disposition, Disposition::Blocked);
    assert_eq!(
        preview.actions[0].reason,
        DispositionReason::UnsupportedWindowsVersion
    );
}

#[test]
fn preview_reports_unknown_platform_when_version_read_fails() {
    let domain = DomainService::new(
        FakeSystemAdapter::new()
            .with_version_failure(ReadFailure::SystemError { code: 1 })
            .with_service(
                "DiagTrack",
                service_state(ServiceRunState::Running, ServiceStartType::Automatic),
            ),
    );

    let preview = completed_preview(domain.preview(&profile()));

    assert_eq!(preview.platform, PlatformSupport::Unknown);
    assert_eq!(preview.actions[0].disposition, Disposition::Unknown);
    assert_eq!(
        preview.actions[0].reason,
        DispositionReason::PlatformUnknown
    );
}

#[test]
fn preview_keeps_other_reads_when_identity_and_power_plan_fail() {
    let domain = DomainService::new(
        FakeSystemAdapter::new()
            .with_identity_failure(ReadFailure::UnsupportedPlatform)
            .with_power_plan_failure(ReadFailure::NotFound)
            .with_service(
                "DiagTrack",
                service_state(ServiceRunState::Running, ServiceStartType::Automatic),
            ),
    );

    let preview = completed_preview(domain.preview(&profile()));

    assert!(matches!(
        preview.interactive_identity,
        Observation::Unavailable {
            failure: ReadFailure::UnsupportedPlatform
        }
    ));
    assert!(matches!(
        preview.active_power_plan,
        Observation::Unavailable {
            failure: ReadFailure::NotFound
        }
    ));
    assert_eq!(preview.actions[0].disposition, Disposition::Applicable);
}

#[test]
fn apply_refuses_when_writes_are_not_implemented() {
    let domain = DomainService::new(FakeSystemAdapter::new());
    let preview_id = PreviewId::parse("preview-1").expect("preview id is valid");

    let outcome = domain.apply(
        &preview_id,
        &Consent {
            preview_id: preview_id.clone(),
        },
    );

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::WriteNotImplemented {
            command: WriteCommand::Apply
        })
    );
}

#[test]
fn apply_refuses_consent_for_a_different_preview() {
    let domain = DomainService::new(FakeSystemAdapter::new());

    let outcome = domain.apply(
        &PreviewId::parse("preview-1").expect("preview id is valid"),
        &Consent {
            preview_id: PreviewId::parse("preview-2").expect("preview id is valid"),
        },
    );

    assert_eq!(outcome, CommandOutcome::Refused(Refusal::ConsentMismatch));
}

#[test]
fn undo_and_restore_all_refuse_when_writes_are_not_implemented() {
    let domain = DomainService::new(FakeSystemAdapter::new());

    assert_eq!(
        domain.undo(&TransactionId::parse("tx-1").expect("transaction id is valid")),
        CommandOutcome::Refused(Refusal::WriteNotImplemented {
            command: WriteCommand::Undo
        })
    );
    assert_eq!(
        domain.restore_all(),
        CommandOutcome::Refused(Refusal::WriteNotImplemented {
            command: WriteCommand::RestoreAll
        })
    );
}

#[test]
fn recovery_status_reports_that_writes_are_unavailable() {
    let domain = DomainService::new(FakeSystemAdapter::new());

    let CommandOutcome::Completed(status) = domain.recovery_status() else {
        panic!("expected recovery status");
    };

    assert_eq!(
        status,
        RecoveryStatus {
            pending_transactions: Vec::new(),
            writes_permitted: false,
            blocker: Some(WriteBlocker::WriteSupportNotImplemented),
        }
    );
}

#[test]
fn unknown_profile_is_refused() {
    let domain = DomainService::new(FakeSystemAdapter::new());

    let outcome = domain.preview(&ProfileId::parse("missing").expect("profile id is valid"));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::UnknownProfile {
            profile_id: "missing".to_owned()
        })
    );
}

#[test]
fn preview_ids_are_unique() {
    let domain = DomainService::new(FakeSystemAdapter::new());

    let first = completed_preview(domain.preview(&profile()));
    let second = completed_preview(domain.preview(&profile()));

    assert_ne!(first.id, second.id);
}
