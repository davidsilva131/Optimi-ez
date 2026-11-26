use std::io::{self, Read, Write};
use std::mem::size_of;

use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::core::PCWSTR;

use optimi_ipc::identity::{
    ClientIdentity, IdentityError, ProcessIdentity, process_creation_time, process_image_path,
    process_user_sid,
};

const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

pub struct PipeServer {
    handle: HANDLE,
}

impl PipeServer {
    pub fn create(pipe_name: &str) -> io::Result<Self> {
        let sddl = wide_string(security_descriptor());
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                windows::Win32::Security::Authorization::SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|_| io::Error::last_os_error())?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let name = wide_string(&format!("\\\\.\\pipe\\{pipe_name}"));
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                PIPE_BUFFER_BYTES,
                PIPE_BUFFER_BYTES,
                0,
                Some(&attributes),
            )
        };
        unsafe {
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
        if handle.is_invalid() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { handle })
    }

    pub fn connect(&self) -> io::Result<()> {
        match unsafe { ConnectNamedPipe(self.handle, None) } {
            Ok(()) => Ok(()),
            Err(error) if error.code().0 as u32 & 0xFFFF == ERROR_PIPE_CONNECTED.0 => Ok(()),
            Err(_) => Err(io::Error::last_os_error()),
        }
    }

    pub fn disconnect(&self) {
        unsafe {
            let _ = DisconnectNamedPipe(self.handle);
        }
    }

    pub fn client(&self) -> io::Result<ClientIdentity> {
        let mut process_id = 0u32;
        unsafe { GetNamedPipeClientProcessId(self.handle, &mut process_id) }
            .map_err(|_| io::Error::last_os_error())?;
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
            .map_err(|_| io::Error::last_os_error())?;
        let client = (|| {
            Ok(ClientIdentity {
                process: ProcessIdentity {
                    process_id,
                    creation_time: process_creation_time(process).map_err(identity)?,
                },
                user_sid: process_user_sid(process).map_err(identity)?,
                image: process_image_path(process).map_err(identity)?,
            })
        })();
        unsafe {
            let _ = CloseHandle(process);
        }
        client
    }

    pub fn stream(&self) -> PipeStream<'_> {
        PipeStream {
            handle: self.handle,
            _scope: std::marker::PhantomData,
        }
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

pub struct PipeStream<'a> {
    handle: HANDLE,
    _scope: std::marker::PhantomData<&'a PipeServer>,
}

impl Read for PipeStream<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut read = 0u32;
        unsafe { ReadFile(self.handle, Some(buffer), Some(&mut read), None) }
            .map_err(|_| io::Error::last_os_error())?;
        if read == 0 && !buffer.is_empty() {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        Ok(read as usize)
    }
}

impl Write for PipeStream<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut written = 0u32;
        unsafe { WriteFile(self.handle, Some(buffer), Some(&mut written), None) }
            .map_err(|_| io::Error::last_os_error())?;
        Ok(written as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn security_descriptor() -> &'static str {
    "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;BU)"
}

fn identity(error: IdentityError) -> io::Error {
    io::Error::other(error)
}

fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipe_descriptor_grants_full_control_only_to_administrators_and_system() {
        let sddl = security_descriptor();

        assert_eq!(sddl, "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;BU)");
        assert!(!sddl.contains("(A;;GA;;;BU)"));
        assert!(!sddl.contains("WD"));
        assert!(!sddl.contains("AN"));
    }

    #[test]
    fn the_pipe_descriptor_is_a_valid_protected_dacl() {
        let sddl = wide_string(security_descriptor());
        let mut descriptor = PSECURITY_DESCRIPTOR::default();

        let parsed = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                windows::Win32::Security::Authorization::SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        };

        assert!(parsed.is_ok());
        assert!(!descriptor.is_invalid());
        unsafe {
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
    }
}
