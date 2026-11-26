use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_SUCCESS, GENERIC_ALL, GENERIC_WRITE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, DACL_SECURITY_INFORMATION,
    GetAce, GetAclInformation, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
    OBJECT_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SE_DACL_PROTECTED, SECURITY_DESCRIPTOR_CONTROL,
};
use windows::Win32::Storage::FileSystem::{
    DELETE, FILE_APPEND_DATA, FILE_DELETE_CHILD, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA,
    FILE_WRITE_EA, WRITE_DAC, WRITE_OWNER,
};
use windows::core::{BOOL, PCWSTR, PWSTR};

const JOURNAL_DIRECTORY: &str = "Optimi-ez";
const JOURNAL_FILE: &str = "journal.json";
const DIRECTORY_SDDL: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)";
const FILE_SDDL: &str = "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;GR;;;BU)";
const LOCAL_SYSTEM_SID: &str = "S-1-5-18";
const ADMINISTRATORS_SID: &str = "S-1-5-32-544";
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const WRITE_RIGHTS: u32 = FILE_WRITE_DATA.0
    | FILE_APPEND_DATA.0
    | FILE_DELETE_CHILD.0
    | FILE_WRITE_EA.0
    | FILE_WRITE_ATTRIBUTES.0
    | DELETE.0
    | WRITE_DAC.0
    | WRITE_OWNER.0
    | GENERIC_WRITE.0
    | GENERIC_ALL.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalSecurityError {
    Io,
    Unprotected,
    WritableByOthers,
    MissingAdministrators,
    MissingSystem,
}

pub fn machine_journal_path() -> PathBuf {
    let root = std::env::var_os("PROGRAMDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\ProgramData"));
    root.join(JOURNAL_DIRECTORY).join(JOURNAL_FILE)
}

pub fn secure_machine_journal(path: &Path) -> Result<(), JournalSecurityError> {
    let directory = path.parent().ok_or(JournalSecurityError::Io)?;
    std::fs::create_dir_all(directory).map_err(|_| JournalSecurityError::Io)?;
    set_dacl(directory, DIRECTORY_SDDL)?;
    if path.exists() {
        set_dacl(path, FILE_SDDL)?;
    }
    verify_machine_journal(path)
}

pub fn verify_machine_journal(path: &Path) -> Result<(), JournalSecurityError> {
    let directory = path.parent().ok_or(JournalSecurityError::Io)?;
    verify(directory)?;
    if path.exists() {
        verify(path)?;
    }
    Ok(())
}

fn set_dacl(path: &Path, sddl: &str) -> Result<(), JournalSecurityError> {
    let name = wide_string(&path.to_string_lossy());
    let sddl = wide_string(sddl);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|_| JournalSecurityError::Io)?;
    let result = (|| {
        let mut present = BOOL(0);
        let mut defaulted = BOOL(0);
        let mut acl: *mut ACL = std::ptr::null_mut();
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) }
            .map_err(|_| JournalSecurityError::Io)?;
        if !present.as_bool() || acl.is_null() {
            return Err(JournalSecurityError::Io);
        }
        let information: OBJECT_SECURITY_INFORMATION =
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
        let status = unsafe {
            SetNamedSecurityInfoW(
                PCWSTR(name.as_ptr()),
                SE_FILE_OBJECT,
                information,
                None,
                None,
                Some(acl),
                None,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(JournalSecurityError::Io);
        }
        Ok(())
    })();
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    result
}

fn verify(path: &Path) -> Result<(), JournalSecurityError> {
    let name = wide_string(&path.to_string_lossy());
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(name.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(JournalSecurityError::Io);
    }
    let result = inspect(descriptor);
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    result
}

fn inspect(descriptor: PSECURITY_DESCRIPTOR) -> Result<(), JournalSecurityError> {
    let mut control = SECURITY_DESCRIPTOR_CONTROL(0);
    let mut revision = 0u32;
    unsafe { GetSecurityDescriptorControl(descriptor, &mut control.0, &mut revision) }
        .map_err(|_| JournalSecurityError::Io)?;
    if !control.contains(SE_DACL_PROTECTED) {
        return Err(JournalSecurityError::Unprotected);
    }
    let mut present = BOOL(0);
    let mut defaulted = BOOL(0);
    let mut acl: *mut ACL = std::ptr::null_mut();
    unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) }
        .map_err(|_| JournalSecurityError::Io)?;
    if !present.as_bool() || acl.is_null() || defaulted.as_bool() {
        return Err(JournalSecurityError::Unprotected);
    }
    let mut size = ACL_SIZE_INFORMATION::default();
    unsafe {
        GetAclInformation(
            acl,
            (&mut size as *mut ACL_SIZE_INFORMATION).cast::<c_void>(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    }
    .map_err(|_| JournalSecurityError::Io)?;
    let mut system_writes = false;
    let mut administrators_write = false;
    for index in 0..size.AceCount {
        let mut entry: *mut c_void = std::ptr::null_mut();
        unsafe { GetAce(acl, index, &mut entry) }.map_err(|_| JournalSecurityError::Io)?;
        if entry.is_null() {
            return Err(JournalSecurityError::Io);
        }
        let allowed = unsafe { &*entry.cast::<ACCESS_ALLOWED_ACE>() };
        if allowed.Header.AceType != ACCESS_ALLOWED_ACE_TYPE {
            continue;
        }
        let sid = PSID(std::ptr::from_ref(&allowed.SidStart).cast_mut().cast());
        let sid = sid_string(sid).map_err(|_| JournalSecurityError::Io)?;
        let writes = allowed.Mask & WRITE_RIGHTS != 0;
        if sid == LOCAL_SYSTEM_SID {
            system_writes |= writes;
        } else if sid == ADMINISTRATORS_SID {
            administrators_write |= writes;
        } else if writes {
            return Err(JournalSecurityError::WritableByOthers);
        }
    }
    if !administrators_write {
        return Err(JournalSecurityError::MissingAdministrators);
    }
    if !system_writes {
        return Err(JournalSecurityError::MissingSystem);
    }
    Ok(())
}

fn sid_string(sid: PSID) -> Result<String, ()> {
    let mut raw = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut raw) }.map_err(|_| ())?;
    let value = unsafe { raw.to_string() }.map_err(|_| ());
    unsafe {
        let _ = LocalFree(Some(HLOCAL(raw.0.cast())));
    }
    value
}

fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch {
        directory: PathBuf,
        journal: PathBuf,
    }

    impl Scratch {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is after the unix epoch")
                .as_nanos();
            let directory = std::env::temp_dir().join(format!(
                "optimi-journal-acl-{}-{unique}",
                std::process::id()
            ));
            let journal = directory.join(JOURNAL_FILE);
            Self { directory, journal }
        }

        fn create(&self) -> &Self {
            std::fs::create_dir_all(&self.directory).expect("scratch directory is created");
            std::fs::write(&self.journal, b"[]").expect("scratch journal is created");
            self
        }

        fn relax(&self, path: &Path, sddl: &str) {
            set_dacl(path, sddl).expect("scratch dacl is replaced");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = set_dacl(&self.directory, "D:P(A;;FA;;;WD)");
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn the_machine_journal_lives_in_program_data() {
        let path = machine_journal_path();

        assert!(path.is_absolute());
        assert!(path.ends_with(Path::new(JOURNAL_DIRECTORY).join(JOURNAL_FILE)));
    }

    #[test]
    fn the_machine_journal_is_writable_only_by_administrators_and_system() {
        let scratch = Scratch::new();
        scratch.create();

        secure_machine_journal(&scratch.journal).expect("the journal is secured");

        verify_machine_journal(&scratch.journal).expect("the secured journal verifies");
    }

    #[test]
    fn a_journal_directory_writable_by_every_user_is_rejected() {
        let scratch = Scratch::new();
        scratch.create();
        secure_machine_journal(&scratch.journal).expect("the journal is secured");

        scratch.relax(&scratch.directory, "D:P(A;;FA;;;WD)");

        assert_eq!(
            verify_machine_journal(&scratch.journal),
            Err(JournalSecurityError::WritableByOthers)
        );
    }

    #[test]
    fn a_journal_file_writable_by_every_user_is_rejected() {
        let scratch = Scratch::new();
        scratch.create();
        secure_machine_journal(&scratch.journal).expect("the journal is secured");

        scratch.relax(&scratch.journal, "D:P(A;;FA;;;WD)");

        assert_eq!(
            verify_machine_journal(&scratch.journal),
            Err(JournalSecurityError::WritableByOthers)
        );
    }

    #[test]
    fn an_inherited_dacl_is_not_accepted_as_protected() {
        let scratch = Scratch::new();
        scratch.create();

        assert_eq!(
            verify_machine_journal(&scratch.journal),
            Err(JournalSecurityError::Unprotected)
        );
    }

    #[test]
    fn the_journal_descriptors_do_not_grant_access_to_every_user() {
        assert!(!DIRECTORY_SDDL.contains("WD"));
        assert!(!DIRECTORY_SDDL.contains(";;;AU"));
        assert!(!FILE_SDDL.contains("WD"));
        assert!(DIRECTORY_SDDL.contains(";;;SY"));
        assert!(FILE_SDDL.contains(";;;BU"));
    }

    #[test]
    fn the_program_data_path_is_built_from_the_environment_root() {
        let root = std::env::var_os("PROGRAMDATA").expect("program data is defined");
        let path = machine_journal_path();

        assert!(path.starts_with(PathBuf::from(root)));
    }
}
