use std::fs::File;
use std::mem::size_of;
use std::os::windows::io::FromRawHandle;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_CANCELLED, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::{GetNamedPipeServerProcessId, WaitNamedPipeW};
use windows::Win32::System::Threading::GetProcessId;
use windows::Win32::UI::Shell::{
    SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::PCWSTR;

use crate::client::{TransportError, WorkerTransport};
use crate::identity::current_user_sid;
use crate::{PIPE_NAME_NONCE_BYTES, pipe_name, read_frame, write_frame};

const CONNECT_ATTEMPTS: usize = 200;
const CONNECT_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub struct PipeSpec {
    pub executable: PathBuf,
    pub journal: PathBuf,
}

pub struct PipeTransport {
    spec: PipeSpec,
    connection: Option<File>,
}

impl PipeTransport {
    pub fn new(spec: PipeSpec) -> Self {
        Self {
            spec,
            connection: None,
        }
    }
}

impl WorkerTransport for PipeTransport {
    fn start(&mut self) -> Result<(), TransportError> {
        if self.connection.is_some() {
            return Ok(());
        }
        let sid = current_user_sid().map_err(|_| TransportError::Unavailable)?;
        let mut nonce = [0u8; PIPE_NAME_NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| TransportError::Unavailable)?;
        let name = pipe_name(&nonce);
        let process_id = launch_elevated(&self.spec, &name, &sid)?;
        let connection = connect(&name, process_id)?;
        self.connection = Some(connection);
        Ok(())
    }

    fn exchange(&mut self, frame: &[u8]) -> Result<Vec<u8>, TransportError> {
        let result = match self.connection.as_mut() {
            Some(connection) => write_frame(connection, frame)
                .map_err(|_| TransportError::Disconnected)
                .and_then(|()| read_frame(connection).map_err(|_| TransportError::Disconnected)),
            None => Err(TransportError::Unavailable),
        };
        if result.is_err() {
            self.connection = None;
        }
        result
    }

    fn close(&mut self) {
        self.connection = None;
    }
}

fn launch_elevated(spec: &PipeSpec, pipe: &str, sid: &str) -> Result<u32, TransportError> {
    let verb = wide_string("runas");
    let file = wide_string(&spec.executable.to_string_lossy());
    let parameters = wide_string(&format!(
        "--pipe {pipe} --journal \"{}\" --consenting-sid {sid} --client-pid {}",
        spec.journal.display(),
        std::process::id()
    ));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(error) = unsafe { ShellExecuteExW(&mut info) } {
        return Err(if win32_code(error) == ERROR_CANCELLED.0 {
            TransportError::ElevationCancelled
        } else {
            TransportError::Unavailable
        });
    }
    if info.hProcess.is_invalid() {
        return Err(TransportError::Unavailable);
    }
    let process_id = unsafe { GetProcessId(info.hProcess) };
    unsafe {
        let _ = CloseHandle(info.hProcess);
    }
    if process_id == 0 {
        return Err(TransportError::Unavailable);
    }
    Ok(process_id)
}

fn connect(name: &str, expected_process_id: u32) -> Result<File, TransportError> {
    let path = wide_string(&format!("\\\\.\\pipe\\{name}"));
    let deadline = Instant::now() + CONNECT_INTERVAL * CONNECT_ATTEMPTS as u32;
    loop {
        unsafe {
            let _ = WaitNamedPipeW(PCWSTR(path.as_ptr()), 100);
        }
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        };
        if let Ok(handle) = handle {
            if server_process_id(handle) == Some(expected_process_id) {
                return Ok(unsafe { File::from_raw_handle(handle.0 as _) });
            }
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(TransportError::Unavailable);
        }
        if Instant::now() >= deadline {
            return Err(TransportError::Unavailable);
        }
        std::thread::sleep(CONNECT_INTERVAL);
    }
}

fn server_process_id(handle: HANDLE) -> Option<u32> {
    let mut process_id = 0u32;
    unsafe { GetNamedPipeServerProcessId(handle, &mut process_id) }
        .ok()
        .filter(|()| process_id != 0)
        .map(|()| process_id)
}

fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn win32_code(error: windows::core::Error) -> u32 {
    let hresult = error.code().0 as u32;
    if hresult & 0xFFFF_0000 == 0x8007_0000 {
        hresult & 0xFFFF
    } else {
        hresult
    }
}
