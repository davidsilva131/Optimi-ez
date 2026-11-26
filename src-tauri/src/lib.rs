pub mod session;

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use optimi_domain::command::{
    AbandonedTransaction, Consent, RecoveryStatus, Refusal, RestoreReport,
};
use optimi_domain::id::{PreviewId, ProfileId, TransactionId};
use optimi_domain::journal::FileJournal;
use optimi_domain::model::{Preview, VerifiedChange};
use optimi_ipc::client::WorkerClient;
use optimi_ipc::pipe::{PipeSpec, PipeTransport};
use optimi_windows::WindowsSystemAdapter;
use session::{Session, UiOutcome};
use tauri::{Manager, State};

struct AppState {
    session: Mutex<Session<WindowsSystemAdapter, FileJournal, WorkerClient<PipeTransport>>>,
}

#[tauri::command]
fn preview_profile(state: State<'_, AppState>, profile_id: String) -> UiOutcome<Preview> {
    match ProfileId::parse(profile_id.as_str()) {
        Ok(profile_id) => lock(&state.session).preview(&profile_id).into(),
        Err(_) => UiOutcome::Refused(Refusal::UnknownProfile { profile_id }),
    }
}

#[tauri::command]
fn apply_preview(
    state: State<'_, AppState>,
    preview_id: String,
    consent_preview_id: String,
) -> UiOutcome<VerifiedChange> {
    let preview_id = match PreviewId::parse(preview_id.as_str()) {
        Ok(preview_id) => preview_id,
        Err(_) => return UiOutcome::Refused(Refusal::InvalidPreviewId { value: preview_id }),
    };
    let consent = match PreviewId::parse(consent_preview_id.as_str()) {
        Ok(consent_id) => Consent {
            preview_id: consent_id,
        },
        Err(_) => {
            return UiOutcome::Refused(Refusal::InvalidPreviewId {
                value: consent_preview_id,
            });
        }
    };
    lock(&state.session).apply(&preview_id, &consent)
}

#[tauri::command]
fn undo_transaction(
    state: State<'_, AppState>,
    transaction_id: String,
) -> UiOutcome<VerifiedChange> {
    match TransactionId::parse(transaction_id.as_str()) {
        Ok(transaction_id) => lock(&state.session).undo(&transaction_id),
        Err(_) => UiOutcome::Refused(Refusal::InvalidTransactionId {
            value: transaction_id,
        }),
    }
}

#[tauri::command]
fn restore_transaction(
    state: State<'_, AppState>,
    transaction_id: String,
) -> UiOutcome<VerifiedChange> {
    match TransactionId::parse(transaction_id.as_str()) {
        Ok(transaction_id) => lock(&state.session).restore(&transaction_id, None),
        Err(_) => UiOutcome::Refused(Refusal::InvalidTransactionId {
            value: transaction_id,
        }),
    }
}

#[tauri::command]
fn restore_after_build_change(
    state: State<'_, AppState>,
    transaction_id: String,
    confirmed_build: u32,
) -> UiOutcome<VerifiedChange> {
    match TransactionId::parse(transaction_id.as_str()) {
        Ok(transaction_id) => lock(&state.session).restore(&transaction_id, Some(confirmed_build)),
        Err(_) => UiOutcome::Refused(Refusal::InvalidTransactionId {
            value: transaction_id,
        }),
    }
}

#[tauri::command]
fn abandon_transaction(
    state: State<'_, AppState>,
    transaction_id: String,
    service: String,
) -> UiOutcome<AbandonedTransaction> {
    match TransactionId::parse(transaction_id.as_str()) {
        Ok(transaction_id) => lock(&state.session).abandon(&transaction_id, service.as_str()),
        Err(_) => UiOutcome::Refused(Refusal::InvalidTransactionId {
            value: transaction_id,
        }),
    }
}

#[tauri::command]
fn restore_all(state: State<'_, AppState>) -> UiOutcome<RestoreReport> {
    lock(&state.session).restore_all()
}

#[tauri::command]
fn recovery_status(state: State<'_, AppState>) -> UiOutcome<RecoveryStatus> {
    lock(&state.session).recovery_status().into()
}

pub fn run() {
    let builder = tauri::Builder::default()
        .setup(|app| {
            let journal_path = app.path().app_data_dir()?.join("journal.json");
            let worker = WorkerClient::new(PipeTransport::new(PipeSpec {
                executable: worker_executable()?,
                journal: journal_path.clone(),
            }));
            let session =
                Session::new(WindowsSystemAdapter, FileJournal::new(journal_path), worker);
            app.manage(AppState {
                session: Mutex::new(session),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            preview_profile,
            apply_preview,
            undo_transaction,
            restore_transaction,
            restore_after_build_change,
            abandon_transaction,
            restore_all,
            recovery_status
        ]);
    let app = builder
        .build(tauri::generate_context!())
        .expect("failed to build Optimi-ez");
    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            if let Some(state) = handle.try_state::<AppState>() {
                lock(&state.session).shutdown();
            }
        }
    });
}

fn worker_executable() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(std::env::current_exe()?.with_file_name("optimi-worker.exe"))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
