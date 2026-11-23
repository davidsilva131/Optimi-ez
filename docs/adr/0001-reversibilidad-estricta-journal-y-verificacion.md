# 0001 — Strict Reversibility Through a Journal and Verification

**Status:** Accepted

## Context

Optimi-ez changes persistent Windows state, sometimes with elevated privileges. An approximate restore can leave Windows different from the state the person approved, especially after an interruption, an external change, or a Windows build update. Reversibility must therefore be a verifiable contract for each action rather than a product promise.

## Decision

Before every external side effect, persist a transaction and a lossless snapshot of every field the typed action will change. After applying or restoring, re-read actual Windows state and verify the result; accepting a request is not success. Keep configuration and runtime status as separate state axes when an action can affect both. An interrupted operation, unknown read, stale state, or mismatch enters `recovery_required` and blocks new writes. Record the Windows build. If it changes, show the diff, mark fields that cannot be verified, and require explicit confirmation before attempting restoration. Never delete an unverified transaction; retain closed transactions until the person deletes them manually.

## Alternatives considered

- **Best-effort undo or recording only that a service was stopped:** rejected because it does not preserve the exact prior values or prove restoration.
- **Using System Restore as the only recovery mechanism:** rejected because it is broad rather than action-specific and does not verify the values changed by an action.
- **Writing the journal after the Windows change or keeping it only in memory:** rejected because a crash between the side effect and the record would leave an unrecoverable change.
- **Treating an accepted restore request as a verified restore:** rejected because only a fresh read of actual state can establish the result.

## Consequences

- Each system adapter must read, snapshot, and verify every field its typed action changes.
- A pending recovery blocks all new writes.
- Action-level undo and `Restore All` follow the same snapshot and read-back rules.
- Transaction states are explicit; a single `active` flag is insufficient.
- The approved domain command seam reports unknown, stale, and mismatched state as a refusal or recovery condition rather than silently overwriting it.
