use std::mem::size_of;

use optimi_domain::journal::JournalKey;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HANDLE};
use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_REGISTRY_KEY};
use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
};
use windows::Win32::Storage::FileSystem::READ_CONTROL;
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS, KEY_READ, KEY_WRITE, REG_BINARY,
    REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW,
    RegQueryValueExW, RegSetValueExW,
};
use windows::core::PCWSTR;

use crate::security::{ACCESS_ALLOWED_ACE_TYPE, SecurityDescriptor, wide_string};

const KEY_PATH: &str = "SOFTWARE\\Optimi-ez";
const KEY_VALUE: &str = "JournalKey";
const KEY_SDDL: &str = "D:P(A;;KA;;;SY)(A;;KA;;;BA)";
const LOCAL_SYSTEM_SID: &str = "S-1-5-18";
const ADMINISTRATORS_SID: &str = "S-1-5-32-544";
const KEY_ACCESS: REG_SAM_FLAGS = REG_SAM_FLAGS(KEY_READ.0 | KEY_WRITE.0 | READ_CONTROL.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalKeyError {
    Io,
    Unavailable,
    InvalidValue,
    Tampered,
}

pub fn load_or_create_journal_key() -> Result<JournalKey, JournalKeyError> {
    let descriptor = SecurityDescriptor::parse(KEY_SDDL).map_err(|_| JournalKeyError::Io)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.raw().0,
        bInheritHandle: false.into(),
    };
    let path = wide_string(KEY_PATH);
    let mut key = HKEY::default();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_ACCESS,
            Some(&attributes),
            &mut key,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(JournalKeyError::Unavailable);
    }
    let result = (|| {
        verify_key_security(key)?;
        let bytes = match read_key_value(key)? {
            Some(bytes) => bytes,
            None => {
                let generated = generate_key()?;
                write_key_value(key, &generated)?;
                let stored = read_key_value(key)?.ok_or(JournalKeyError::Unavailable)?;
                if stored != generated {
                    return Err(JournalKeyError::Unavailable);
                }
                stored.to_vec()
            }
        };
        validate_key_value(&bytes)?;
        Ok(JournalKey::new(
            bytes
                .try_into()
                .map_err(|_| JournalKeyError::InvalidValue)?,
        ))
    })();
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

fn verify_key_security(key: HKEY) -> Result<(), JournalKeyError> {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetSecurityInfo(
            HANDLE(key.0),
            SE_REGISTRY_KEY,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            Some(&mut descriptor),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(JournalKeyError::Tampered);
    }
    let descriptor = SecurityDescriptor::from_raw(descriptor);
    inspect_key_descriptor(&descriptor)
}

fn inspect_key_descriptor(descriptor: &SecurityDescriptor) -> Result<(), JournalKeyError> {
    if !descriptor
        .is_protected()
        .map_err(|_| JournalKeyError::Tampered)?
    {
        return Err(JournalKeyError::Tampered);
    }
    let aces = descriptor.aces().map_err(|_| JournalKeyError::Tampered)?;
    let mut system = false;
    let mut administrators = false;
    for ace in aces {
        if ace.ace_type != ACCESS_ALLOWED_ACE_TYPE {
            return Err(JournalKeyError::Tampered);
        }
        let sid = ace.sid.as_deref().ok_or(JournalKeyError::Tampered)?;
        if sid == LOCAL_SYSTEM_SID {
            if ace.mask & KEY_ALL_ACCESS.0 != KEY_ALL_ACCESS.0 {
                return Err(JournalKeyError::Tampered);
            }
            system = true;
        } else if sid == ADMINISTRATORS_SID {
            if ace.mask & KEY_ALL_ACCESS.0 != KEY_ALL_ACCESS.0 {
                return Err(JournalKeyError::Tampered);
            }
            administrators = true;
        } else {
            return Err(JournalKeyError::Tampered);
        }
    }
    if !system || !administrators {
        return Err(JournalKeyError::Tampered);
    }
    Ok(())
}

fn read_key_value(key: HKEY) -> Result<Option<Vec<u8>>, JournalKeyError> {
    let name = wide_string(KEY_VALUE);
    let mut kind = REG_VALUE_TYPE::default();
    let mut length = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut length),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS || kind != REG_BINARY {
        return Err(JournalKeyError::InvalidValue);
    }
    let mut bytes = vec![0u8; length as usize];
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some(bytes.as_mut_ptr()),
            Some(&mut length),
        )
    };
    if status != ERROR_SUCCESS || kind != REG_BINARY {
        return Err(JournalKeyError::InvalidValue);
    }
    bytes.truncate(length as usize);
    Ok(Some(bytes))
}

fn write_key_value(key: HKEY, bytes: &[u8]) -> Result<(), JournalKeyError> {
    let name = wide_string(KEY_VALUE);
    let status =
        unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_BINARY, Some(bytes)) };
    if status != ERROR_SUCCESS {
        return Err(JournalKeyError::Unavailable);
    }
    Ok(())
}

fn validate_key_value(bytes: &[u8]) -> Result<(), JournalKeyError> {
    if bytes.len() != JournalKey::LENGTH {
        return Err(JournalKeyError::InvalidValue);
    }
    Ok(())
}

fn generate_key() -> Result<[u8; JournalKey::LENGTH], JournalKeyError> {
    let mut bytes = [0u8; JournalKey::LENGTH];
    let status = unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status.0 < 0 {
        return Err(JournalKeyError::Io);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(sddl: &str) -> SecurityDescriptor {
        SecurityDescriptor::parse(sddl).expect("descriptor parses")
    }

    #[test]
    fn the_key_descriptor_is_readable_and_writable_only_by_system_and_administrators() {
        inspect_key_descriptor(&descriptor(KEY_SDDL)).expect("key descriptor is accepted");

        assert!(!KEY_SDDL.contains(";;;BU"));
        assert!(!KEY_SDDL.contains("WD"));
        assert!(!KEY_SDDL.contains(";;;AU"));
    }

    #[test]
    fn a_key_descriptor_writable_by_users_is_rejected() {
        assert_eq!(
            inspect_key_descriptor(&descriptor("D:P(A;;KA;;;SY)(A;;KA;;;BA)(A;;KW;;;BU)")),
            Err(JournalKeyError::Tampered)
        );
    }

    #[test]
    fn a_key_descriptor_readable_by_users_is_rejected() {
        assert_eq!(
            inspect_key_descriptor(&descriptor("D:P(A;;KA;;;SY)(A;;KA;;;BA)(A;;KR;;;BU)")),
            Err(JournalKeyError::Tampered)
        );
    }

    #[test]
    fn an_inherited_key_descriptor_is_rejected() {
        assert_eq!(
            inspect_key_descriptor(&descriptor("D:(A;;KA;;;SY)(A;;KA;;;BA)")),
            Err(JournalKeyError::Tampered)
        );
    }

    #[test]
    fn a_key_descriptor_missing_administrators_is_rejected() {
        assert_eq!(
            inspect_key_descriptor(&descriptor("D:P(A;;KA;;;SY)")),
            Err(JournalKeyError::Tampered)
        );
    }

    #[test]
    fn a_key_value_of_the_wrong_length_is_rejected() {
        assert_eq!(
            validate_key_value(&[0u8; JournalKey::LENGTH - 1]),
            Err(JournalKeyError::InvalidValue)
        );
        assert_eq!(
            validate_key_value(&[0u8; JournalKey::LENGTH + 1]),
            Err(JournalKeyError::InvalidValue)
        );
        assert_eq!(validate_key_value(&[0u8; JournalKey::LENGTH]), Ok(()));
    }

    #[test]
    fn generated_keys_are_random_and_have_the_expected_length() {
        let first = generate_key().expect("the system random generator works");
        let second = generate_key().expect("the system random generator works");

        assert_eq!(first.len(), JournalKey::LENGTH);
        assert_ne!(first, second);
    }
}
