use std::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSidToSidW};
use windows::Win32::Security::{GetTokenInformation, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PWSTR;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityError {
    Token,
    Sid,
}

pub fn current_user_sid() -> Result<String, IdentityError> {
    process_user_sid(unsafe { GetCurrentProcess() })
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

pub fn canonical_sid(value: &str) -> Result<String, IdentityError> {
    let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let mut sid = PSID(std::ptr::null_mut());
    unsafe { ConvertStringSidToSidW(windows::core::PCWSTR(wide.as_ptr()), &mut sid) }
        .map_err(|_| IdentityError::Sid)?;
    let result = sid_string(sid);
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sid.0)));
    }
    result
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
