use serde::Serialize;

use crate::id::{ActionId, PreviewId, ProfileId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementStatus {
    Measured,
    NotMeasured,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    pub source: String,
    pub url: String,
    pub date: String,
    pub measurement: MeasurementStatus,
    pub claim_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceRunState {
    Stopped,
    StartPending,
    StopPending,
    Running,
    ContinuePending,
    PausePending,
    Paused,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStartType {
    Boot,
    System,
    Automatic,
    Manual,
    Disabled,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ServiceState {
    pub run_state: ServiceRunState,
    pub start_type: ServiceStartType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsProductType {
    Workstation,
    Server,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WindowsVersion {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    pub product_type: WindowsProductType,
}

impl WindowsVersion {
    pub fn supports_current_release(&self) -> bool {
        self.major >= 10
            && self.build >= 22000
            && self.product_type == WindowsProductType::Workstation
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WindowsIdentity {
    pub account: String,
    pub domain: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActivePowerPlan {
    pub guid: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadFailure {
    AccessDenied,
    NotFound,
    UnsupportedPlatform,
    InvalidData,
    SystemError { code: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Observation<T> {
    Observed { value: T },
    Unavailable { failure: ReadFailure },
}

impl<T> Observation<T> {
    pub fn observed(value: T) -> Self {
        Self::Observed { value }
    }

    pub fn unavailable(failure: ReadFailure) -> Self {
        Self::Unavailable { failure }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformSupport {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Applicable,
    NoOp,
    Blocked,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DispositionReason {
    TargetDiffers,
    AlreadyAtTarget,
    StateUnavailable,
    UnsupportedWindowsVersion,
    PlatformUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionPreview {
    pub action_id: ActionId,
    pub service: String,
    pub risk: Risk,
    pub evidence: Evidence,
    pub target_start_type: ServiceStartType,
    pub observed: Observation<ServiceState>,
    pub disposition: Disposition,
    pub reason: DispositionReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preview {
    pub id: PreviewId,
    pub profile_id: ProfileId,
    pub catalog_version: String,
    pub platform: PlatformSupport,
    pub windows_version: Observation<WindowsVersion>,
    pub interactive_identity: Observation<WindowsIdentity>,
    pub active_power_plan: Observation<ActivePowerPlan>,
    pub actions: Vec<ActionPreview>,
}
