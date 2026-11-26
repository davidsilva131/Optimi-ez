use std::ffi::c_void;
use std::fmt;
use std::path::PathBuf;

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcessToken, PROCESS_NAME_WIN32,
    QueryFullProcessImageNameW,
};
use windows::core::PWSTR;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityError {
    Token,
    Sid,
    Times,
    Image,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token => formatter.write_str("process token is unavailable"),
            Self::Sid => formatter.write_str("process identity is unreadable"),
            Self::Times => formatter.write_str("process creation time is unreadable"),
            Self::Image => formatter.write_str("process image path is unreadable"),
        }
    }
}

impl std::error::Error for IdentityError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub process_id: u32,
    pub creation_time: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIdentity {
    pub process: ProcessIdentity,
    pub user_sid: String,
    pub image: PathBuf,
}

pub fn process_user_sid(process: HANDLE) -> Result<String, IdentityError> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }
        .map_err(|_| IdentityError::Token)?;
    let result = token_user_sid(token);
    unsafe {
        let _ = CloseHandle(token);
    }
    result
}

pub fn current_process_creation_time() -> Result<u64, IdentityError> {
    process_creation_time(unsafe { GetCurrentProcess() })
}

pub fn process_creation_time(process: HANDLE) -> Result<u64, IdentityError> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
        .map_err(|_| IdentityError::Times)?;
    Ok(u64::from(creation.dwHighDateTime) << 32 | u64::from(creation.dwLowDateTime))
}

pub fn process_image_path(process: HANDLE) -> Result<PathBuf, IdentityError> {
    let mut buffer = vec![0u16; 32 * 1024];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(|_| IdentityError::Image)?;
    buffer.truncate(length as usize);
    String::from_utf16(&buffer)
        .map(PathBuf::from)
        .map_err(|_| IdentityError::Image)
}

fn token_user_sid(token: HANDLE) -> Result<String, IdentityError> {
    let mut needed = 0u32;
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    if needed == 0 {
        return Err(IdentityError::Token);
    }
    let mut buffer = vec![0u8; needed as usize];
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            needed,
            &mut needed,
        )
    }
    .map_err(|_| IdentityError::Token)?;
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    sid_string(user.User.Sid)
}

fn sid_string(sid: PSID) -> Result<String, IdentityError> {
    let mut raw = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut raw) }.map_err(|_| IdentityError::Sid)?;
    let value = unsafe { raw.to_string() }.map_err(|_| IdentityError::Sid);
    unsafe {
        let _ = LocalFree(Some(HLOCAL(raw.0.cast())));
    }
    value
}
