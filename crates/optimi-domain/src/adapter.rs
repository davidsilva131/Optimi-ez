use crate::model::{
    ActivePowerPlan, AntiCheatState, ReadFailure, ServiceState, WindowsIdentity, WindowsVersion,
    WriteFailure,
};

pub trait SystemAdapter: Send + Sync {
    fn windows_version(&self) -> Result<WindowsVersion, ReadFailure>;
    fn interactive_identity(&self) -> Result<WindowsIdentity, ReadFailure>;
    fn service_state(&self, service: &str) -> Result<ServiceState, ReadFailure>;
    fn active_power_plan(&self) -> Result<ActivePowerPlan, ReadFailure>;
    fn anti_cheat_state(&self) -> Result<AntiCheatState, ReadFailure>;
    fn write_service_state(&self, service: &str, target: ServiceState) -> Result<(), WriteFailure>;
}
