use serde::{Deserialize, Serialize};

use crate::id::{ActionId, PreviewId, ProfileId, TransactionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementStatus {
    Measured,
    NotMeasured,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub source: String,
    pub url: String,
    pub date: String,
    pub measurement: MeasurementStatus,
    pub claim_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

impl ServiceRunState {
    pub fn is_settled(&self) -> bool {
        matches!(self, Self::Running | Self::Stopped)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStartType {
    Boot,
    System,
    Automatic,
    Manual,
    Disabled,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceState {
    pub run_state: ServiceRunState,
    pub start_type: ServiceStartType,
    pub delayed_start: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsProductType {
    Workstation,
    Server,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsIdentity {
    pub account: String,
    pub domain: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivePowerPlan {
    pub guid: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadFailure {
    AccessDenied,
    NotFound,
    UnsupportedPlatform,
    InvalidData,
    SystemError { code: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WriteFailure {
    AccessDenied,
    NotFound,
    UnsupportedTarget,
    Timeout,
    SystemError { code: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    Platform,
    AntiCheat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AntiCheatState {
    pub active: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformSupport {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Applicable,
    NoOp,
    Blocked,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispositionReason {
    TargetDiffers,
    AlreadyAtTarget,
    StateUnavailable,
    UnsupportedWindowsVersion,
    PlatformUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPreview {
    pub action_id: ActionId,
    pub service: String,
    pub risk: Risk,
    pub evidence: Evidence,
    pub target: ServiceState,
    pub observed: Observation<ServiceState>,
    pub disposition: Disposition,
    pub reason: DispositionReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSnapshot {
    pub service: String,
    pub existed: bool,
    pub state: Option<ServiceState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedChange {
    pub transaction_id: TransactionId,
    pub action_id: ActionId,
    pub service: String,
    pub verified_state: ServiceState,
}
