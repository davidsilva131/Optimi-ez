use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use optimi_domain::command::DomainService;
use optimi_domain::journal::FileJournal;
use optimi_ipc::{FrameReadError, MAX_FRAME_BYTES, ProtocolError, read_frame, write_frame};
use optimi_windows::WindowsSystemAdapter;

use crate::WorkerFailure;
use crate::config::Config;
use crate::dispatch::{WorkerCommands, dispatch, rejection};
use crate::pipe_server::PipeServer;

const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(30);

pub fn run(config: Config) -> Result<(), WorkerFailure> {
    let server = PipeServer::create(&config.pipe_name, &config.consenting_sid)
        .map_err(|_| WorkerFailure::Pipe)?;
    let commands = DomainService::new(WindowsSystemAdapter, FileJournal::new(config.journal));
    let activity = Arc::new(AtomicU64::new(now_millis()));
    spawn_watchdog(Arc::clone(&activity));
    loop {
        server.connect().map_err(|_| WorkerFailure::Pipe)?;
        if !server.client_is_consenting_user(&config.consenting_sid, config.client_process_id) {
            server.disconnect();
            continue;
        }
        activity.store(now_millis(), Ordering::Relaxed);
        let result = serve_connection(&commands, &server, &activity);
        server.disconnect();
        return result;
    }
}

fn serve_connection<C: WorkerCommands>(
    commands: &C,
    server: &PipeServer,
    activity: &AtomicU64,
) -> Result<(), WorkerFailure> {
    let mut stream = server.stream();
    loop {
        match read_frame(&mut stream) {
            Ok(frame) => {
                activity.store(now_millis(), Ordering::Relaxed);
                let response = dispatch(commands, &frame);
                if write_frame(&mut stream, &response).is_err() {
                    return Ok(());
                }
            }
            Err(FrameReadError::TooLarge) => {
                let response = rejection(ProtocolError::FrameTooLarge {
                    limit: MAX_FRAME_BYTES,
                });
                let _ = write_frame(&mut stream, &response);
                return Ok(());
            }
            Err(FrameReadError::Empty) => {
                let response = rejection(ProtocolError::EmptyFrame);
                let _ = write_frame(&mut stream, &response);
                return Ok(());
            }
            Err(FrameReadError::Io) => return Ok(()),
        }
    }
}

fn spawn_watchdog(activity: Arc<AtomicU64>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(WATCHDOG_INTERVAL);
            let idle = now_millis().saturating_sub(activity.load(Ordering::Relaxed));
            if idle >= IDLE_TIMEOUT.as_millis() as u64 {
                std::process::exit(0);
            }
        }
    });
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
