#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use optimi_domain::command::{CommandOutcome, Consent, DomainService, RecoveryStatus, Refusal};
use optimi_domain::id::{PreviewId, ProfileId, TransactionId};
use optimi_domain::model::Preview;
use optimi_windows::WindowsSystemAdapter;
use tauri::State;

struct AppState {
    domain: DomainService<WindowsSystemAdapter>,
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
) -> CommandOutcome<TransactionId> {
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
fn undo_transaction(state: State<'_, AppState>, transaction_id: String) -> CommandOutcome<()> {
    match TransactionId::parse(transaction_id.as_str()) {
        Ok(transaction_id) => state.domain.undo(&transaction_id),
        Err(_) => CommandOutcome::Refused(Refusal::InvalidTransactionId {
            value: transaction_id,
        }),
    }
}

#[tauri::command]
fn restore_all(state: State<'_, AppState>) -> CommandOutcome<Vec<TransactionId>> {
    state.domain.restore_all()
}

#[tauri::command]
fn recovery_status(state: State<'_, AppState>) -> CommandOutcome<RecoveryStatus> {
    state.domain.recovery_status()
}

fn main() {
    let domain = DomainService::new(WindowsSystemAdapter);
    println!("optimi-ez: read-only domain surface ready");
    tauri::Builder::default()
        .manage(AppState { domain })
        .invoke_handler(tauri::generate_handler![
            preview_profile,
            apply_preview,
            undo_transaction,
            restore_all,
            recovery_status
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Optimi-ez");
}
