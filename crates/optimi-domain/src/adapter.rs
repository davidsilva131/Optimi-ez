use crate::model::{ActivePowerPlan, ReadFailure, ServiceState, WindowsIdentity, WindowsVersion};

pub trait SystemAdapter {
    fn windows_version(&self) -> Result<WindowsVersion, ReadFailure>;
    fn interactive_identity(&self) -> Result<WindowsIdentity, ReadFailure>;
    fn service_state(&self, service: &str) -> Result<ServiceState, ReadFailure>;
    fn active_power_plan(&self) -> Result<ActivePowerPlan, ReadFailure>;
}
