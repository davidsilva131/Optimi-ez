use std::mem::size_of;

use optimi_domain::adapter::SystemAdapter;
use optimi_domain::model::{
    ActivePowerPlan, ReadFailure, ServiceRunState, ServiceStartType, ServiceState, WindowsIdentity,
    WindowsProductType, WindowsVersion,
};
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS, HLOCAL, LocalFree};
use windows::Win32::System::Power::{PowerGetActiveScheme, PowerReadFriendlyName};
use windows::Win32::System::RemoteDesktop::{
    WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTS_INFO_CLASS, WTSDomainName, WTSFreeMemory,
    WTSQuerySessionInformationW, WTSUserName,
};
use windows::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QUERY_SERVICE_CONFIGW, QueryServiceConfigW,
    QueryServiceStatus, SC_HANDLE, SC_MANAGER_CONNECT, SERVICE_AUTO_START, SERVICE_BOOT_START,
    SERVICE_CONTINUE_PENDING, SERVICE_DEMAND_START, SERVICE_DISABLED, SERVICE_PAUSE_PENDING,
    SERVICE_PAUSED, SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS, SERVICE_RUNNING,
    SERVICE_START_PENDING, SERVICE_START_TYPE, SERVICE_STATUS, SERVICE_STATUS_CURRENT_STATE,
    SERVICE_STOP_PENDING, SERVICE_STOPPED, SERVICE_SYSTEM_START,
};
use windows::Win32::System::SystemInformation::{OSVERSIONINFOEXW, OSVERSIONINFOW};
use windows::core::{GUID, PCWSTR, PWSTR};

const PRODUCT_TYPE_WORKSTATION: u8 = 1;
const PRODUCT_TYPE_DOMAIN_CONTROLLER: u8 = 2;
const PRODUCT_TYPE_SERVER: u8 = 3;

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsSystemAdapter;

impl SystemAdapter for WindowsSystemAdapter {
    fn windows_version(&self) -> Result<WindowsVersion, ReadFailure> {
        let mut info = OSVERSIONINFOEXW {
            dwOSVersionInfoSize: size_of::<OSVERSIONINFOEXW>() as u32,
            ..Default::default()
        };
        let status =
            unsafe { RtlGetVersion(std::ptr::from_mut(&mut info).cast::<OSVERSIONINFOW>()) };
        if status.0 < 0 {
            return Err(ReadFailure::SystemError {
                code: status.0 as u32,
            });
        }
        Ok(WindowsVersion {
            major: info.dwMajorVersion,
            minor: info.dwMinorVersion,
            build: info.dwBuildNumber,
            product_type: product_type(info.wProductType),
        })
    }

    fn interactive_identity(&self) -> Result<WindowsIdentity, ReadFailure> {
        let account = wts_string(WTSUserName)?;
        let domain = wts_string(WTSDomainName)?;
        Ok(WindowsIdentity {
            account,
            domain: (!domain.is_empty()).then_some(domain),
        })
    }

    fn service_state(&self, service: &str) -> Result<ServiceState, ReadFailure> {
        let manager = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
            .map_err(map_error)?;
        let name = wide_string(service);
        let handle = match unsafe {
            OpenServiceW(
                manager,
                PCWSTR(name.as_ptr()),
                SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG,
            )
        } {
            Ok(handle) => handle,
            Err(error) => {
                unsafe {
                    let _ = CloseServiceHandle(manager);
                }
                return Err(map_error(error));
            }
        };
        let result = read_service_state(handle);
        unsafe {
            let _ = CloseServiceHandle(handle);
            let _ = CloseServiceHandle(manager);
        }
        result
    }

    fn active_power_plan(&self) -> Result<ActivePowerPlan, ReadFailure> {
        let mut pointer: *mut GUID = std::ptr::null_mut();
        let status = unsafe { PowerGetActiveScheme(None, &mut pointer) };
        if status != ERROR_SUCCESS {
            return Err(map_win32(status.0));
        }
        if pointer.is_null() {
            return Err(ReadFailure::InvalidData);
        }
        let guid = unsafe { *pointer };
        unsafe {
            LocalFree(Some(HLOCAL(pointer.cast())));
        }
        Ok(ActivePowerPlan {
            guid: format_guid(&guid),
            name: read_plan_name(&guid)?,
        })
    }
}

fn wts_string(class: WTS_INFO_CLASS) -> Result<String, ReadFailure> {
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    unsafe {
        WTSQuerySessionInformationW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            WTS_CURRENT_SESSION,
            class,
            &mut buffer,
            &mut bytes,
        )
    }
    .map_err(map_error)?;
    let value = unsafe { buffer.to_string() }.map_err(|_| ReadFailure::InvalidData);
    unsafe {
        WTSFreeMemory(buffer.0.cast());
    }
    value
}

fn read_service_state(handle: SC_HANDLE) -> Result<ServiceState, ReadFailure> {
    let mut status = SERVICE_STATUS::default();
    unsafe { QueryServiceStatus(handle, &mut status) }.map_err(map_error)?;

    let mut needed = 0u32;
    let _ = unsafe { QueryServiceConfigW(handle, None, 0, &mut needed) };
    if needed == 0 {
        return Err(ReadFailure::InvalidData);
    }
    let mut buffer = vec![0u64; (needed as usize).div_ceil(size_of::<u64>())];
    unsafe {
        QueryServiceConfigW(
            handle,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
    }
    .map_err(map_error)?;
    let config = unsafe { &*buffer.as_ptr().cast::<QUERY_SERVICE_CONFIGW>() };

    Ok(ServiceState {
        run_state: run_state(status.dwCurrentState),
        start_type: start_type(config.dwStartType),
    })
}

fn run_state(state: SERVICE_STATUS_CURRENT_STATE) -> ServiceRunState {
    if state.0 == SERVICE_STOPPED.0 {
        ServiceRunState::Stopped
    } else if state.0 == SERVICE_START_PENDING.0 {
        ServiceRunState::StartPending
    } else if state.0 == SERVICE_STOP_PENDING.0 {
        ServiceRunState::StopPending
    } else if state.0 == SERVICE_RUNNING.0 {
        ServiceRunState::Running
    } else if state.0 == SERVICE_CONTINUE_PENDING.0 {
        ServiceRunState::ContinuePending
    } else if state.0 == SERVICE_PAUSE_PENDING.0 {
        ServiceRunState::PausePending
    } else if state.0 == SERVICE_PAUSED.0 {
        ServiceRunState::Paused
    } else {
        ServiceRunState::Unknown
    }
}

fn start_type(value: SERVICE_START_TYPE) -> ServiceStartType {
    if value.0 == SERVICE_BOOT_START.0 {
        ServiceStartType::Boot
    } else if value.0 == SERVICE_SYSTEM_START.0 {
        ServiceStartType::System
    } else if value.0 == SERVICE_AUTO_START.0 {
        ServiceStartType::Automatic
    } else if value.0 == SERVICE_DEMAND_START.0 {
        ServiceStartType::Manual
    } else if value.0 == SERVICE_DISABLED.0 {
        ServiceStartType::Disabled
    } else {
        ServiceStartType::Unknown
    }
}

fn read_plan_name(guid: &GUID) -> Result<Option<String>, ReadFailure> {
    let mut size = 0u32;
    let status = unsafe { PowerReadFriendlyName(None, Some(guid), None, None, None, &mut size) };
    if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
        return Err(map_win32(status.0));
    }
    if size == 0 {
        return Ok(None);
    }
    let mut buffer = vec![0u8; size as usize];
    let status = unsafe {
        PowerReadFriendlyName(
            None,
            Some(guid),
            None,
            None,
            Some(buffer.as_mut_ptr()),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(map_win32(status.0));
    }
    let units: Vec<u16> = buffer
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    let name = String::from_utf16(&units).map_err(|_| ReadFailure::InvalidData)?;
    Ok(Some(name))
}

fn format_guid(guid: &GUID) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7]
    )
}

fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn product_type(raw: u8) -> WindowsProductType {
    match raw {
        PRODUCT_TYPE_WORKSTATION => WindowsProductType::Workstation,
        PRODUCT_TYPE_DOMAIN_CONTROLLER | PRODUCT_TYPE_SERVER => WindowsProductType::Server,
        _ => WindowsProductType::Unknown,
    }
}

fn map_error(error: windows::core::Error) -> ReadFailure {
    let hresult = error.code().0 as u32;
    if hresult & 0xFFFF_0000 == 0x8007_0000 {
        return map_win32(hresult & 0xFFFF);
    }
    ReadFailure::SystemError { code: hresult }
}

fn map_win32(code: u32) -> ReadFailure {
    match code {
        5 => ReadFailure::AccessDenied,
        2 | 3 | 1060 => ReadFailure::NotFound,
        _ => ReadFailure::SystemError { code },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_windows_product_type() {
        assert_eq!(product_type(1), WindowsProductType::Workstation);
        assert_eq!(product_type(2), WindowsProductType::Server);
        assert_eq!(product_type(3), WindowsProductType::Server);
        assert_eq!(product_type(0), WindowsProductType::Unknown);
    }
}
