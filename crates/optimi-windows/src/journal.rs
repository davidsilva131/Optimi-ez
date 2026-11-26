use std::mem::size_of;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, ERROR_FILE_NOT_FOUND,
    ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, GENERIC_WRITE, HANDLE, LUID,
};
use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo};
use windows::Win32::Security::{
    AdjustTokenPrivileges, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, LUID_AND_ATTRIBUTES,
    LookupPrivilegeValueW, OBJECT_INHERIT_ACE, OBJECT_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SE_PRIVILEGE_ENABLED, SE_TAKE_OWNERSHIP_NAME, SECURITY_ATTRIBUTES, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW, DELETE, FILE_ALL_ACCESS,
    FILE_APPEND_DATA, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_DELETE_CHILD, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FILE_WRITE_EA,
    GetFileInformationByHandle, OPEN_EXISTING, READ_CONTROL, WRITE_DAC, WRITE_OWNER, WriteFile,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{PCWSTR, Result as WindowsResult};

use crate::security::{
    ACCESS_ALLOWED_ACE_TYPE, Ace, SecurityDescriptor, SecurityId, sid_string, wide_string,
};

const JOURNAL_DIRECTORY: &str = "Optimi-ez";
const JOURNAL_FILE: &str = "journal.json";
const DIRECTORY_SDDL: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)";
const FILE_SDDL: &str = "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;GR;;;BU)";
const LOCAL_SYSTEM_SID: &str = "S-1-5-18";
const ADMINISTRATORS_SID: &str = "S-1-5-32-544";
const USERS_SID: &str = "S-1-5-32-545";
const WRITE_RIGHTS: u32 = FILE_WRITE_DATA.0
    | FILE_APPEND_DATA.0
    | FILE_DELETE_CHILD.0
    | FILE_WRITE_EA.0
    | FILE_WRITE_ATTRIBUTES.0
    | DELETE.0
    | WRITE_DAC.0
    | WRITE_OWNER.0
    | GENERIC_WRITE.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalSecurityError {
    Io,
    Unprotected,
    WritableByOthers,
    MissingAdministrators,
    MissingSystem,
    OwnerUntrusted,
    OwnershipUnavailable,
    ReparsePoint,
    HardLinked,
    HostileTemporary,
}

trait OwnershipPolicy {
    fn is_trusted(&self, path: &Path) -> Result<bool, JournalSecurityError>;
    fn take(&self, path: &Path) -> Result<(), JournalSecurityError>;
}

struct SystemOwnership;

impl OwnershipPolicy for SystemOwnership {
    fn is_trusted(&self, path: &Path) -> Result<bool, JournalSecurityError> {
        let handle = open_verified(path, READ_CONTROL.0)?;
        let mut owner = PSID::default();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let status = unsafe {
            GetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                Some(&mut owner),
                None,
                None,
                None,
                Some(&mut descriptor),
            )
        };
        let result = (|| {
            if status != ERROR_SUCCESS {
                return Err(JournalSecurityError::Io);
            }
            sid_string(owner)
                .map(|sid| trusted_owner(&sid))
                .map_err(|_| JournalSecurityError::Io)
        })();
        close(handle);
        free_descriptor(descriptor);
        result
    }

    fn take(&self, path: &Path) -> Result<(), JournalSecurityError> {
        enable_take_ownership_privilege();
        let owner = SecurityId::parse(ADMINISTRATORS_SID)
            .map_err(|_| JournalSecurityError::OwnershipUnavailable)?;
        let handle = open_object(path, WRITE_OWNER.0 | READ_CONTROL.0)
            .map_err(|_| JournalSecurityError::OwnershipUnavailable)?;
        let status = unsafe {
            SetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                Some(owner.as_psid()),
                None,
                None,
                None,
            )
        };
        close(handle);
        if status != ERROR_SUCCESS {
            return Err(JournalSecurityError::OwnershipUnavailable);
        }
        Ok(())
    }
}

pub fn machine_journal_path() -> PathBuf {
    let root = std::env::var_os("PROGRAMDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\ProgramData"));
    root.join(JOURNAL_DIRECTORY).join(JOURNAL_FILE)
}

pub fn secure_machine_journal(path: &Path) -> Result<(), JournalSecurityError> {
    secure_with(path, &SystemOwnership)
}

pub fn verify_machine_journal(path: &Path) -> Result<(), JournalSecurityError> {
    verify_with(path, &SystemOwnership)
}

fn secure_with(path: &Path, ownership: &dyn OwnershipPolicy) -> Result<(), JournalSecurityError> {
    ensure_no_reparse(path)?;
    let directory = path.parent().ok_or(JournalSecurityError::Io)?;
    secure_directory(directory, ownership)?;
    secure_file(path, ownership)?;
    secure_temporary(&path.with_extension("tmp"), ownership)?;
    verify_with(path, ownership)
}

fn verify_with(path: &Path, ownership: &dyn OwnershipPolicy) -> Result<(), JournalSecurityError> {
    ensure_no_reparse(path)?;
    let directory = path.parent().ok_or(JournalSecurityError::Io)?;
    verify_kind(directory, ownership, ObjectKind::Directory)?;
    if path.exists() {
        verify_kind(path, ownership, ObjectKind::File)?;
    }
    let temporary = path.with_extension("tmp");
    if temporary.exists() {
        verify_kind(&temporary, ownership, ObjectKind::File)?;
    }
    Ok(())
}

fn secure_directory(
    directory: &Path,
    ownership: &dyn OwnershipPolicy,
) -> Result<(), JournalSecurityError> {
    ensure_no_reparse(directory)?;
    let _ = create_directory(directory)?;
    ensure_owner(directory, ownership)?;
    set_dacl(directory, DIRECTORY_SDDL)?;
    verify_kind(directory, ownership, ObjectKind::Directory)
}

fn secure_file(path: &Path, ownership: &dyn OwnershipPolicy) -> Result<(), JournalSecurityError> {
    if create_file(path, FILE_SDDL, Some(b"[]"))? == Creation::Exists {
        open_verified(path, READ_CONTROL.0)?;
    }
    ensure_owner(path, ownership)?;
    set_dacl(path, FILE_SDDL)?;
    verify_kind(path, ownership, ObjectKind::File)
}

fn secure_temporary(
    path: &Path,
    ownership: &dyn OwnershipPolicy,
) -> Result<(), JournalSecurityError> {
    match create_file(path, FILE_SDDL, None)? {
        Creation::Created => {}
        Creation::Exists => return reject_hostile_temporary(path, ownership),
    }
    verify_kind(path, ownership, ObjectKind::File)
}

fn reject_hostile_temporary(
    path: &Path,
    ownership: &dyn OwnershipPolicy,
) -> Result<(), JournalSecurityError> {
    let handle =
        open_verified(path, READ_CONTROL.0).map_err(|_| JournalSecurityError::HostileTemporary)?;
    let result = if ownership.is_trusted(path).unwrap_or(false) {
        inspect_handle(handle, ObjectKind::File).map_err(|_| JournalSecurityError::HostileTemporary)
    } else {
        Err(JournalSecurityError::HostileTemporary)
    };
    close(handle);
    result
}

fn ensure_owner(path: &Path, ownership: &dyn OwnershipPolicy) -> Result<(), JournalSecurityError> {
    if ownership.is_trusted(path)? {
        return Ok(());
    }
    ownership.take(path)?;
    if ownership.is_trusted(path)? {
        Ok(())
    } else {
        Err(JournalSecurityError::OwnershipUnavailable)
    }
}

fn verify_kind(
    path: &Path,
    ownership: &dyn OwnershipPolicy,
    kind: ObjectKind,
) -> Result<(), JournalSecurityError> {
    let handle = open_verified(path, READ_CONTROL.0)?;
    let result = (|| {
        if !ownership.is_trusted(path)? {
            return Err(JournalSecurityError::OwnerUntrusted);
        }
        inspect_handle(handle, kind)
    })();
    close(handle);
    result
}

fn inspect_handle(handle: HANDLE, kind: ObjectKind) -> Result<(), JournalSecurityError> {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            Some(&mut descriptor),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(JournalSecurityError::Io);
    }
    let descriptor = SecurityDescriptor::from_raw(descriptor);
    inspect_dacl(&descriptor, kind)
}

fn inspect_dacl(
    descriptor: &SecurityDescriptor,
    kind: ObjectKind,
) -> Result<(), JournalSecurityError> {
    if !descriptor
        .is_protected()
        .map_err(|_| JournalSecurityError::Unprotected)?
    {
        return Err(JournalSecurityError::Unprotected);
    }
    let aces = descriptor
        .aces()
        .map_err(|_| JournalSecurityError::Unprotected)?;
    let mut system = false;
    let mut administrators = false;
    let mut users = false;
    for ace in aces {
        if ace.ace_type != ACCESS_ALLOWED_ACE_TYPE {
            return Err(JournalSecurityError::Unprotected);
        }
        let sid = ace
            .sid
            .as_deref()
            .ok_or(JournalSecurityError::Unprotected)?;
        if sid == LOCAL_SYSTEM_SID {
            require_full(&ace, kind)?;
            system = true;
        } else if sid == ADMINISTRATORS_SID {
            require_full(&ace, kind)?;
            administrators = true;
        } else if ace.mask & WRITE_RIGHTS != 0 {
            return Err(JournalSecurityError::WritableByOthers);
        } else if sid == USERS_SID {
            users = true;
        }
    }
    if !system {
        return Err(JournalSecurityError::MissingSystem);
    }
    if !administrators {
        return Err(JournalSecurityError::MissingAdministrators);
    }
    if !users {
        return Err(JournalSecurityError::Unprotected);
    }
    Ok(())
}

fn require_full(ace: &Ace, kind: ObjectKind) -> Result<(), JournalSecurityError> {
    if ace.mask & FILE_ALL_ACCESS.0 != FILE_ALL_ACCESS.0 {
        return Err(if ace.sid.as_deref() == Some(LOCAL_SYSTEM_SID) {
            JournalSecurityError::MissingSystem
        } else {
            JournalSecurityError::MissingAdministrators
        });
    }
    let inherited = OBJECT_INHERIT_ACE.0 | CONTAINER_INHERIT_ACE.0;
    if kind == ObjectKind::Directory && ace.flags & inherited != inherited {
        return Err(JournalSecurityError::Unprotected);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectKind {
    Directory,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Creation {
    Created,
    Exists,
}

fn create_directory(path: &Path) -> Result<Creation, JournalSecurityError> {
    let descriptor =
        SecurityDescriptor::parse(DIRECTORY_SDDL).map_err(|_| JournalSecurityError::Io)?;
    let attributes = security_attributes(&descriptor);
    let name = wide_string(&path.to_string_lossy());
    match unsafe { CreateDirectoryW(PCWSTR(name.as_ptr()), Some(&attributes)) } {
        Ok(()) => Ok(Creation::Created),
        Err(error) if win32_code(&error) == ERROR_ALREADY_EXISTS.0 => Ok(Creation::Exists),
        Err(_) => Err(JournalSecurityError::Io),
    }
}

fn create_file(
    path: &Path,
    sddl: &str,
    contents: Option<&[u8]>,
) -> Result<Creation, JournalSecurityError> {
    let descriptor = SecurityDescriptor::parse(sddl).map_err(|_| JournalSecurityError::Io)?;
    let attributes = security_attributes(&descriptor);
    let name = wide_string(&path.to_string_lossy());
    let handle = match unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            GENERIC_WRITE.0 | READ_CONTROL.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            Some(&attributes),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    } {
        Ok(handle) => handle,
        Err(error)
            if win32_code(&error) == ERROR_FILE_EXISTS.0
                || win32_code(&error) == ERROR_ALREADY_EXISTS.0 =>
        {
            return Ok(Creation::Exists);
        }
        Err(_) => return Err(JournalSecurityError::Io),
    };
    let result = match contents {
        Some(data) => unsafe { WriteFile(handle, Some(data), None, None) }
            .map_err(|_| JournalSecurityError::Io),
        None => Ok(()),
    };
    close(handle);
    result?;
    Ok(Creation::Created)
}

fn set_dacl(path: &Path, sddl: &str) -> Result<(), JournalSecurityError> {
    let descriptor = SecurityDescriptor::parse(sddl).map_err(|_| JournalSecurityError::Io)?;
    let acl = descriptor.acl().map_err(|_| JournalSecurityError::Io)?;
    let handle =
        open_object(path, WRITE_DAC.0 | READ_CONTROL.0).map_err(|_| JournalSecurityError::Io)?;
    let information: OBJECT_SECURITY_INFORMATION =
        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
    let status = unsafe {
        SetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            information,
            None,
            None,
            Some(acl),
            None,
        )
    };
    close(handle);
    if status != ERROR_SUCCESS {
        return Err(JournalSecurityError::Io);
    }
    Ok(())
}

fn ensure_no_reparse(path: &Path) -> Result<(), JournalSecurityError> {
    let mut components = Vec::new();
    for ancestor in path.ancestors() {
        if ancestor.as_os_str().is_empty() {
            break;
        }
        components.push(ancestor.to_path_buf());
    }
    for component in components.iter().rev() {
        match open_object(component, FILE_READ_ATTRIBUTES.0 | READ_CONTROL.0) {
            Ok(handle) => {
                let result = reject_reparse(handle);
                close(handle);
                result?;
            }
            Err(error) if is_missing(&error) => {}
            Err(_) => return Err(JournalSecurityError::Io),
        }
    }
    Ok(())
}

fn open_verified(path: &Path, access: u32) -> Result<HANDLE, JournalSecurityError> {
    let handle = open_object(path, access).map_err(|_| JournalSecurityError::Io)?;
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let result = (|| {
        unsafe { GetFileInformationByHandle(handle, &mut information) }
            .map_err(|_| JournalSecurityError::Io)?;
        if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(JournalSecurityError::ReparsePoint);
        }
        let directory = information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
        if !directory && information.nNumberOfLinks != 1 {
            return Err(JournalSecurityError::HardLinked);
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(handle),
        Err(error) => {
            close(handle);
            Err(error)
        }
    }
}

fn reject_reparse(handle: HANDLE) -> Result<(), JournalSecurityError> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle, &mut information) }
        .map_err(|_| JournalSecurityError::Io)?;
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(JournalSecurityError::ReparsePoint);
    }
    Ok(())
}

fn open_object(path: &Path, access: u32) -> WindowsResult<HANDLE> {
    let name = wide_string(&path.to_string_lossy());
    unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
}

fn security_attributes(descriptor: &SecurityDescriptor) -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.raw().0,
        bInheritHandle: false.into(),
    }
}

fn trusted_owner(sid: &str) -> bool {
    sid == LOCAL_SYSTEM_SID || sid == ADMINISTRATORS_SID
}

fn enable_take_ownership_privilege() {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .is_err()
        {
            return;
        }
        let mut luid = LUID::default();
        if LookupPrivilegeValueW(None, SE_TAKE_OWNERSHIP_NAME, &mut luid).is_ok() {
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            let _ = AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None);
        }
        close(token);
    }
}

fn close(handle: HANDLE) {
    unsafe {
        let _ = CloseHandle(handle);
    }
}

fn free_descriptor(descriptor: PSECURITY_DESCRIPTOR) {
    unsafe {
        let _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            descriptor.0,
        )));
    }
}

fn is_missing(error: &windows::core::Error) -> bool {
    let code = win32_code(error);
    code == ERROR_FILE_NOT_FOUND.0 || code == ERROR_PATH_NOT_FOUND.0
}

fn win32_code(error: &windows::core::Error) -> u32 {
    let hresult = error.code().0 as u32;
    if hresult & 0xFFFF_0000 == 0x8007_0000 {
        hresult & 0xFFFF
    } else {
        hresult
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::process::Command;

    use super::*;

    struct FakeOwnership {
        untrusted: RefCell<HashSet<PathBuf>>,
        taken: RefCell<Vec<PathBuf>>,
        may_take: bool,
    }

    impl FakeOwnership {
        fn new() -> Self {
            Self {
                untrusted: RefCell::new(HashSet::new()),
                taken: RefCell::new(Vec::new()),
                may_take: true,
            }
        }

        fn distrust(&self, path: &Path) {
            self.untrusted.borrow_mut().insert(path.to_path_buf());
        }

        fn took(&self, path: &Path) -> bool {
            self.taken.borrow().iter().any(|taken| taken == path)
        }
    }

    impl OwnershipPolicy for FakeOwnership {
        fn is_trusted(&self, path: &Path) -> Result<bool, JournalSecurityError> {
            Ok(!self.untrusted.borrow().contains(path))
        }

        fn take(&self, path: &Path) -> Result<(), JournalSecurityError> {
            if !self.may_take {
                return Err(JournalSecurityError::OwnershipUnavailable);
            }
            self.untrusted.borrow_mut().remove(path);
            self.taken.borrow_mut().push(path.to_path_buf());
            Ok(())
        }
    }

    struct Scratch {
        directory: PathBuf,
        journal: PathBuf,
        temporary: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is after the unix epoch")
                .as_nanos();
            let directory = std::env::temp_dir().join(format!(
                "optimi-journal-acl-{name}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&directory).expect("scratch directory is created");
            let journal = directory.join(JOURNAL_FILE);
            let temporary = journal.with_extension("tmp");
            std::fs::write(&journal, b"[]").expect("scratch journal is created");
            std::fs::write(&temporary, b"").expect("scratch temporary is created");
            Self {
                directory,
                journal,
                temporary,
            }
        }

        fn prepare(&self) {
            self.prepare_temporary();
            self.prepare_directory();
        }

        fn prepare_without_temporary(&self) {
            set_dacl(&self.journal, FILE_SDDL).expect("journal dacl is applied");
            set_dacl(&self.directory, DIRECTORY_SDDL).expect("directory dacl is applied");
        }

        fn prepare_temporary(&self) {
            set_dacl(&self.journal, FILE_SDDL).expect("journal dacl is applied");
            set_dacl(&self.temporary, FILE_SDDL).expect("temporary dacl is applied");
        }

        fn prepare_directory(&self) {
            set_dacl(&self.directory, DIRECTORY_SDDL).expect("directory dacl is applied");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = set_dacl(&self.journal, "D:P(A;;FA;;;WD)");
            let _ = set_dacl(&self.temporary, "D:P(A;;FA;;;WD)");
            let _ = set_dacl(&self.directory, "D:P(A;;FA;;;WD)");
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn create_junction(link: &Path, target: &Path) {
        let command = std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into());
        let status = Command::new(command)
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .status()
            .expect("mklink runs");
        assert!(status.success(), "junction is created");
    }

    #[test]
    fn the_machine_journal_lives_in_program_data() {
        let path = machine_journal_path();

        assert!(path.is_absolute());
        assert!(path.ends_with(Path::new(JOURNAL_DIRECTORY).join(JOURNAL_FILE)));
    }

    #[test]
    fn the_journal_directory_and_files_are_locked_to_administrators_and_system() {
        let scratch = Scratch::new("locked");
        scratch.prepare();
        let ownership = FakeOwnership::new();

        secure_with(&scratch.journal, &ownership).expect("the journal is secured");
        verify_with(&scratch.journal, &ownership).expect("the secured journal verifies");

        assert!(std::fs::write(&scratch.journal, b"[]").is_err());
        assert!(std::fs::write(scratch.directory.join("intruder.txt"), b"x").is_err());
    }

    #[test]
    fn a_user_owned_directory_is_taken_over_before_the_dacl_is_applied() {
        let scratch = Scratch::new("takeover");
        scratch.prepare();
        let ownership = FakeOwnership::new();
        ownership.distrust(&scratch.directory);

        secure_with(&scratch.journal, &ownership).expect("the journal is secured");

        assert!(ownership.took(&scratch.directory));
        assert!(std::fs::write(&scratch.journal, b"[]").is_err());
    }

    #[test]
    fn when_ownership_cannot_be_taken_the_worker_refuses_to_serve() {
        let scratch = Scratch::new("refused");
        let ownership = FakeOwnership {
            may_take: false,
            ..FakeOwnership::new()
        };
        ownership.distrust(&scratch.directory);

        assert_eq!(
            secure_with(&scratch.journal, &ownership),
            Err(JournalSecurityError::OwnershipUnavailable)
        );
        assert!(
            std::fs::write(scratch.directory.join("probe.txt"), b"x").is_ok(),
            "a refused takeover must not lock the directory"
        );
    }

    #[test]
    fn the_real_owner_check_rejects_a_directory_created_by_the_current_user() {
        let scratch = Scratch::new("real-owner");

        assert!(
            !SystemOwnership
                .is_trusted(&scratch.directory)
                .expect("owner is readable")
        );

        match secure_machine_journal(&scratch.journal) {
            Ok(()) => assert!(
                SystemOwnership
                    .is_trusted(&scratch.directory)
                    .expect("owner is readable")
            ),
            Err(error) => assert_eq!(error, JournalSecurityError::OwnershipUnavailable),
        }
    }

    #[test]
    fn a_reparse_point_in_the_journal_path_is_rejected() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "optimi-journal-junction-{}-{unique}",
            std::process::id()
        ));
        let target = root.join("target");
        let link = root.join("link");
        std::fs::create_dir_all(&target).expect("junction target is created");
        create_junction(&link, &target);
        let journal = link.join(JOURNAL_FILE);
        let ownership = FakeOwnership::new();

        let result = secure_with(&journal, &ownership);

        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(result, Err(JournalSecurityError::ReparsePoint));
    }

    #[test]
    fn a_temporary_file_owned_by_another_principal_is_rejected() {
        let scratch = Scratch::new("hostile-owner");
        scratch.prepare();
        let ownership = FakeOwnership::new();
        ownership.distrust(&scratch.temporary);

        assert_eq!(
            secure_with(&scratch.journal, &ownership),
            Err(JournalSecurityError::HostileTemporary)
        );
        assert!(!ownership.took(&scratch.temporary));
    }

    #[test]
    fn a_temporary_file_with_a_foreign_dacl_is_rejected() {
        let scratch = Scratch::new("hostile-dacl");
        scratch.prepare_without_temporary();
        let ownership = FakeOwnership::new();

        assert_eq!(
            secure_with(&scratch.journal, &ownership),
            Err(JournalSecurityError::HostileTemporary)
        );
    }

    #[test]
    fn a_hard_linked_temporary_file_is_rejected() {
        let scratch = Scratch::new("hardlink");
        let target = scratch.directory.join("link-target.json");
        std::fs::write(&target, b"").expect("hardlink target is created");
        std::fs::remove_file(&scratch.temporary).expect("scratch temporary is removed");
        std::fs::hard_link(&target, &scratch.temporary).expect("temporary hardlink is created");
        scratch.prepare_temporary();
        scratch.prepare_directory();
        let ownership = FakeOwnership::new();

        assert_eq!(
            secure_with(&scratch.journal, &ownership),
            Err(JournalSecurityError::HostileTemporary)
        );
        let _ = set_dacl(&target, "D:P(A;;FA;;;WD)");
    }

    #[test]
    fn a_relaxed_directory_dacl_is_rejected_by_the_handle_check() {
        let scratch = Scratch::new("relaxed-directory");
        scratch.prepare();
        let ownership = FakeOwnership::new();
        secure_with(&scratch.journal, &ownership).expect("the journal is secured");

        set_dacl(&scratch.directory, "D:P(A;;FA;;;WD)").expect("directory dacl is relaxed");

        assert_eq!(
            verify_with(&scratch.journal, &ownership),
            Err(JournalSecurityError::WritableByOthers)
        );
    }

    #[test]
    fn a_relaxed_journal_dacl_is_rejected_by_the_handle_check() {
        let scratch = Scratch::new("relaxed-journal");
        scratch.prepare();
        let ownership = FakeOwnership::new();
        secure_with(&scratch.journal, &ownership).expect("the journal is secured");

        set_dacl(&scratch.journal, "D:P(A;;FA;;;WD)").expect("journal dacl is relaxed");

        assert_eq!(
            verify_with(&scratch.journal, &ownership),
            Err(JournalSecurityError::WritableByOthers)
        );
    }

    #[test]
    fn an_inherited_dacl_is_not_accepted_as_protected() {
        let scratch = Scratch::new("inherited");
        let ownership = FakeOwnership::new();

        assert_eq!(
            verify_with(&scratch.journal, &ownership),
            Err(JournalSecurityError::Unprotected)
        );
    }

    #[test]
    fn journal_and_temporary_files_are_created_with_the_restricted_descriptor() {
        let scratch = Scratch::new("create");
        std::fs::remove_file(&scratch.journal).expect("scratch journal is removed");
        std::fs::remove_file(&scratch.temporary).expect("scratch temporary is removed");
        let ownership = FakeOwnership::new();

        assert_eq!(
            create_file(&scratch.journal, FILE_SDDL, Some(b"[]")).expect("journal is created"),
            Creation::Created
        );
        assert_eq!(
            create_file(&scratch.temporary, FILE_SDDL, None).expect("temporary is created"),
            Creation::Created
        );
        verify_kind(&scratch.journal, &ownership, ObjectKind::File)
            .expect("created journal verifies");
        verify_kind(&scratch.temporary, &ownership, ObjectKind::File)
            .expect("created temporary verifies");
        assert!(std::fs::write(&scratch.journal, b"[]").is_err());
        assert!(std::fs::write(&scratch.temporary, b"tamper").is_err());
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
