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

const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

pub struct PipeServer {
    handle: HANDLE,
}

impl PipeServer {
    pub fn create(pipe_name: &str, consenting_sid: &str) -> io::Result<Self> {
        let sddl = wide_string(&security_descriptor(consenting_sid));
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

    pub fn client_is_consenting_user(&self, expected_sid: &str, expected_process_id: u32) -> bool {
        let mut process_id = 0u32;
        if unsafe { GetNamedPipeClientProcessId(self.handle, &mut process_id) }.is_err()
            || process_id != expected_process_id
        {
            return false;
        }
        let process =
            match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) } {
                Ok(process) => process,
                Err(_) => return false,
            };
        let sid = optimi_ipc::identity::process_user_sid(process).ok();
        unsafe {
            let _ = CloseHandle(process);
        }
        sid.as_deref() == Some(expected_sid)
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

pub fn security_descriptor(consenting_sid: &str) -> String {
    format!("D:P(A;;GA;;;BA)(A;;GA;;;{consenting_sid})")
}

fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipe_grants_only_administrators_and_the_consenting_user() {
        let sddl = security_descriptor("S-1-5-21-1004336348-1177238915-682003330-1001");

        assert_eq!(
            sddl,
            "D:P(A;;GA;;;BA)(A;;GA;;;S-1-5-21-1004336348-1177238915-682003330-1001)"
        );
        assert!(!sddl.contains("WD"));
        assert!(!sddl.contains("AN"));
    }
}
