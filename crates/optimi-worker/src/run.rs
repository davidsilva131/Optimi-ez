use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use optimi_domain::command::DomainService;
use optimi_domain::journal::FileJournal;
use optimi_ipc::identity::current_process_creation_time;
use optimi_ipc::{FrameReadError, MAX_FRAME_BYTES, ProtocolError, read_frame, write_frame};
use optimi_windows::WindowsSystemAdapter;
use optimi_windows::journal::{machine_journal_path, secure_machine_journal};
use optimi_windows::journal_key::load_or_create_journal_key;

use crate::WorkerFailure;
use crate::config::Config;
use crate::dispatch::{WorkerCommands, dispatch, rejection};
use crate::pipe_server::PipeServer;
use crate::session::{Session, SessionPolicy};

const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(30);
const ACCEPTANCE_BACKOFF_INITIAL: Duration = Duration::from_millis(10);
const ACCEPTANCE_BACKOFF_MAX: Duration = Duration::from_secs(1);
const CLIENT_EXECUTABLE: &str = "optimi-ez.exe";

pub fn run(config: Config) -> Result<(), WorkerFailure> {
    let journal = machine_journal_path();
    secure_machine_journal(&journal).map_err(|_| WorkerFailure::Journal)?;
    let key = load_or_create_journal_key().map_err(|_| WorkerFailure::Journal)?;
    let policy = SessionPolicy::new(client_executable()?, started_at()?);
    let server = PipeServer::create(&config.pipe_name).map_err(|_| WorkerFailure::Pipe)?;
    let commands = DomainService::new(WindowsSystemAdapter, FileJournal::new(journal, key));
    let activity = Arc::new(AtomicU64::new(now_millis()));
    spawn_watchdog(Arc::clone(&activity));
    let mut backoff = AcceptanceBackoff::new();
    loop {
        server.connect().map_err(|_| WorkerFailure::Pipe)?;
        let authorized = server
            .client()
            .ok()
            .and_then(|client| policy.authorize(client));
        let Some(session) = authorized else {
            server.disconnect();
            std::thread::sleep(backoff.rejected());
            continue;
        };
        backoff.accepted();
        activity.store(now_millis(), Ordering::Relaxed);
        let result = serve_connection(&commands, &server, &session, &activity);
        server.disconnect();
        return result;
    }
}

fn serve_connection<C: WorkerCommands>(
    commands: &C,
    server: &PipeServer,
    session: &Session,
    activity: &AtomicU64,
) -> Result<(), WorkerFailure> {
    let mut stream = server.stream();
    loop {
        if !server.client().is_ok_and(|client| session.allows(&client)) {
            return Ok(());
        }
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

fn client_executable() -> Result<PathBuf, WorkerFailure> {
    std::env::current_exe()
        .map(|path| path.with_file_name(CLIENT_EXECUTABLE))
        .map_err(|_| WorkerFailure::Identity)
}

fn started_at() -> Result<u64, WorkerFailure> {
    current_process_creation_time().map_err(|_| WorkerFailure::Identity)
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

struct AcceptanceBackoff {
    next: Duration,
}

impl AcceptanceBackoff {
    fn new() -> Self {
        Self {
            next: ACCEPTANCE_BACKOFF_INITIAL,
        }
    }

    fn rejected(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(ACCEPTANCE_BACKOFF_MAX);
        delay
    }

    fn accepted(&mut self) {
        self.next = ACCEPTANCE_BACKOFF_INITIAL;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rejected_client_waits_with_exponential_backoff_up_to_the_cap() {
        let mut backoff = AcceptanceBackoff::new();

        assert_eq!(backoff.rejected(), Duration::from_millis(10));
        assert_eq!(backoff.rejected(), Duration::from_millis(20));
        assert_eq!(backoff.rejected(), Duration::from_millis(40));
        assert_eq!(backoff.rejected(), Duration::from_millis(80));
        assert_eq!(backoff.rejected(), Duration::from_millis(160));
        assert_eq!(backoff.rejected(), Duration::from_millis(320));
        assert_eq!(backoff.rejected(), Duration::from_millis(640));
        assert_eq!(backoff.rejected(), ACCEPTANCE_BACKOFF_MAX);
        assert_eq!(backoff.rejected(), ACCEPTANCE_BACKOFF_MAX);
    }

    #[test]
    fn an_authorized_client_resets_the_acceptance_backoff() {
        let mut backoff = AcceptanceBackoff::new();
        backoff.rejected();
        backoff.rejected();
        backoff.rejected();

        backoff.accepted();

        assert_eq!(backoff.rejected(), ACCEPTANCE_BACKOFF_INITIAL);
    }
}
