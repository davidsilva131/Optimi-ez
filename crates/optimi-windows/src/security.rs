use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    ConvertStringSidToSidW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, GetAce, GetAclInformation,
    GetSecurityDescriptorControl, GetSecurityDescriptorDacl, PSECURITY_DESCRIPTOR, PSID,
    SE_DACL_PROTECTED, SECURITY_DESCRIPTOR_CONTROL,
};
use windows::core::{BOOL, PCWSTR, PWSTR};

pub(crate) const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

pub(crate) struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl SecurityDescriptor {
    pub(crate) fn parse(sddl: &str) -> Result<Self, ()> {
        let text = wide_string(sddl);
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(text.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|_| ())?;
        Ok(Self(descriptor))
    }

    pub(crate) fn raw(&self) -> PSECURITY_DESCRIPTOR {
        self.0
    }

    pub(crate) fn from_raw(descriptor: PSECURITY_DESCRIPTOR) -> Self {
        Self(descriptor)
    }

    pub(crate) fn is_protected(&self) -> Result<bool, ()> {
        let mut control = SECURITY_DESCRIPTOR_CONTROL(0);
        let mut revision = 0u32;
        unsafe { GetSecurityDescriptorControl(self.0, &mut control.0, &mut revision) }
            .map_err(|_| ())?;
        Ok(control.contains(SE_DACL_PROTECTED))
    }

    pub(crate) fn acl(&self) -> Result<*mut ACL, ()> {
        let mut present = BOOL(0);
        let mut defaulted = BOOL(0);
        let mut acl: *mut ACL = std::ptr::null_mut();
        unsafe { GetSecurityDescriptorDacl(self.0, &mut present, &mut acl, &mut defaulted) }
            .map_err(|_| ())?;
        if !present.as_bool() || acl.is_null() || defaulted.as_bool() {
            return Err(());
        }
        Ok(acl)
    }

    pub(crate) fn aces(&self) -> Result<Vec<Ace>, ()> {
        let acl = self.acl()?;
        let mut size = ACL_SIZE_INFORMATION::default();
        unsafe {
            GetAclInformation(
                acl,
                std::ptr::from_mut(&mut size).cast::<c_void>(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        }
        .map_err(|_| ())?;
        let mut aces = Vec::with_capacity(size.AceCount as usize);
        for index in 0..size.AceCount {
            let mut entry: *mut c_void = std::ptr::null_mut();
            unsafe { GetAce(acl, index, &mut entry) }.map_err(|_| ())?;
            if entry.is_null() {
                return Err(());
            }
            let ace = unsafe { &*entry.cast::<ACCESS_ALLOWED_ACE>() };
            if ace.Header.AceType != ACCESS_ALLOWED_ACE_TYPE {
                aces.push(Ace {
                    ace_type: ace.Header.AceType,
                    mask: 0,
                    flags: u32::from(ace.Header.AceFlags),
                    sid: None,
                });
                continue;
            }
            let sid = PSID(std::ptr::from_ref(&ace.SidStart).cast_mut().cast());
            aces.push(Ace {
                ace_type: ace.Header.AceType,
                mask: ace.Mask,
                flags: u32::from(ace.Header.AceFlags),
                sid: Some(sid_string(sid)?),
            });
        }
        Ok(aces)
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0.0)));
        }
    }
}

#[derive(Debug)]
pub(crate) struct Ace {
    pub(crate) ace_type: u8,
    pub(crate) mask: u32,
    pub(crate) flags: u32,
    pub(crate) sid: Option<String>,
}

pub(crate) struct SecurityId(PSID);

impl SecurityId {
    pub(crate) fn parse(value: &str) -> Result<Self, ()> {
        let text = wide_string(value);
        let mut sid = PSID::default();
        unsafe { ConvertStringSidToSidW(PCWSTR(text.as_ptr()), &mut sid) }.map_err(|_| ())?;
        Ok(Self(sid))
    }

    pub(crate) fn as_psid(&self) -> PSID {
        self.0
    }
}

impl Drop for SecurityId {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0.0)));
        }
    }
}

pub(crate) fn sid_string(sid: PSID) -> Result<String, ()> {
    let mut raw = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut raw) }.map_err(|_| ())?;
    let value = unsafe { raw.to_string() }.map_err(|_| ());
    unsafe {
        let _ = LocalFree(Some(HLOCAL(raw.0.cast())));
    }
    value
}

pub(crate) fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
