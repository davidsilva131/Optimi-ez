#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use optimi_domain::command::{
    AbandonConsent, AbandonedTransaction, BuildChangeConfirmation, CommandOutcome, Consent,
    DomainService, RecoveryStatus, Refusal, RestoreReport,
};
use optimi_domain::id::{PreviewId, ProfileId, TransactionId};
use optimi_domain::journal::FileJournal;
use optimi_domain::model::{Preview, VerifiedChange};
use optimi_windows::WindowsSystemAdapter;
use tauri::{Manager, State};

struct AppState {
    domain: DomainService<WindowsSystemAdapter, FileJournal>,
}

#[tauri::command]
fn preview_profile(state: State<'_, AppState>, profile_id: String) -> CommandOutcome<Preview> {
    match ProfileId::parse(profile_id.as_str()) {
        Ok(profile_id) => state.domain.preview(&profile_id),
        Err(_) => CommandOutcome::Refused(Refusal::UnknownProfile { profile_id }),
    }
}

#[tauri::command]
fn apply_preview(
    state: State<'_, AppState>,
    preview_id: String,
    consent_preview_id: String,
) -> CommandOutcome<VerifiedChange> {
    let preview_id = match PreviewId::parse(preview_id.as_str()) {
        Ok(preview_id) => preview_id,
        Err(_) => return CommandOutcome::Refused(Refusal::InvalidPreviewId { value: preview_id }),
    };
    let consent = match PreviewId::parse(consent_preview_id.as_str()) {
        Ok(consent_id) => Consent {
            preview_id: consent_id,
        },
        Err(_) => {
            return CommandOutcome::Refused(Refusal::InvalidPreviewId {
                value: consent_preview_id,
            });
        }
    };
    state.domain.apply(&preview_id, &consent)
}

#[tauri::command]
fn undo_transaction(
    state: State<'_, AppState>,
    transaction_id: String,
) -> CommandOutcome<VerifiedChange> {
    match TransactionId::parse(transaction_id.as_str()) {
        Ok(transaction_id) => state.domain.undo(&transaction_id),
        Err(_) => CommandOutcome::Refused(Refusal::InvalidTransactionId {
            value: transaction_id,
        }),
    }
}

#[tauri::command]
fn restore_transaction(
    state: State<'_, AppState>,
    transaction_id: String,
) -> CommandOutcome<VerifiedChange> {
    match TransactionId::parse(transaction_id.as_str()) {
        Ok(transaction_id) => state.domain.restore_transaction(&transaction_id),
        Err(_) => CommandOutcome::Refused(Refusal::InvalidTransactionId {
            value: transaction_id,
        }),
    }
}

#[tauri::command]
fn restore_after_build_change(
    state: State<'_, AppState>,
    transaction_id: String,
    confirmed_build: u32,
) -> CommandOutcome<VerifiedChange> {
    match TransactionId::parse(transaction_id.as_str()) {
        Ok(transaction_id) => state
            .domain
            .restore_after_build_change(&BuildChangeConfirmation {
                transaction_id,
                confirmed_build,
            }),
        Err(_) => CommandOutcome::Refused(Refusal::InvalidTransactionId {
            value: transaction_id,
        }),
    }
}

#[tauri::command]
fn abandon_transaction(
    state: State<'_, AppState>,
    transaction_id: String,
    service: String,
) -> CommandOutcome<AbandonedTransaction> {
    match TransactionId::parse(transaction_id.as_str()) {
        Ok(transaction_id) => state.domain.abandon(&AbandonConsent {
            transaction_id,
            service,
        }),
        Err(_) => CommandOutcome::Refused(Refusal::InvalidTransactionId {
            value: transaction_id,
        }),
    }
}

#[tauri::command]
fn restore_all(state: State<'_, AppState>) -> CommandOutcome<RestoreReport> {
    state.domain.restore_all()
}

#[tauri::command]
fn recovery_status(state: State<'_, AppState>) -> CommandOutcome<RecoveryStatus> {
    state.domain.recovery_status()
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let journal_path = app.path().app_data_dir()?.join("journal.json");
            let domain = DomainService::new(WindowsSystemAdapter, FileJournal::new(journal_path));
            app.manage(AppState { domain });
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
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Optimi-ez");
}
