# 0005 — Journal Integrity and the Machine Key

**Status:** Accepted

## Context

The machine journal persists snapshots and transaction state that an elevated worker may use to restore Windows service state. Its protected DACL limits ordinary-user writes, but filesystem access control alone does not tell the worker whether a record it reads is authentic. The worker needs a durable, machine-scoped integrity check before it accepts journal state for a privileged write or restore.

## Decision

Each durable journal record is authenticated individually with HMAC-SHA256 over its serialized record. The 32-byte key is generated with the system random-number generator and stored as the `REG_BINARY` value `JournalKey` under `HKLM\SOFTWARE\Optimi-ez`. The registry key has the protected DACL `D:P(A;;KA;;;SY)(A;;KA;;;BA)`: `SYSTEM` and `BUILTIN\Administrators` have full access, and the unprivileged UI user cannot read the key.

When the elevated worker reads the journal for an operation, every record must contain a valid MAC. A missing, malformed, or invalid MAC invalidates the entire journal and fails closed; the worker does not accept a valid prefix or skip a bad record. The read-only UI view is for display and does not authorize writes; only the worker's authenticated journal read participates in a privileged operation.

The MACs are per-record, not chained: the current format has no sequence number or `prev_mac` link.

## Alternatives considered

- **Machine-wide DPAPI:** not selected. The chosen format verifies individual journal records and defines an explicit whole-journal failure rule when any record is invalid. DPAPI would be a different protected-blob design; this decision does not claim that DPAPI is incapable of providing authenticated protection.
- **Keeping the journal only in memory:** rejected because the worker or app can exit or the machine can restart before recovery, and the pending transaction would be lost.
- **Relying on the journal DACL alone:** rejected because access control does not authenticate record contents or detect a modified record once it is read. The DACL remains necessary as a separate access-control layer.

## Consequences

- HMAC detects changes to an individual record when the key remains unavailable to the writer. It does not encrypt the journal: `BUILTIN\Users` can read the machine journal, but cannot read the key from an unelevated process.
- One invalid record makes the whole journal unusable for worker operations. This avoids partial acceptance at the cost of recovery availability when any record is corrupted.
- Because records are not chained, copying, reordering, or removing otherwise valid records does not invalidate their individual MACs. A follow-up must add authenticated `seq` and `prev_mac` linkage and a protected expected journal head (or equivalent) so that removal of a valid suffix is also detected; a chain alone cannot detect tail truncation.
- The directory owner and DACL are verified when the worker starts, not before every write. A later DACL change is not detected by the running worker. Reparse-point and hard-link checks are also not repeated before each runtime write, and the actual object type of each opened handle is not validated; these remain tracked residuals.
- The journal DACL validator currently permits additional read-only ACEs. Whether that is acceptable is an unresolved policy decision tracked for follow-up.
- This integrity scheme is not a defense against an elevated attacker who can read or replace the machine key. A person who can obtain UAC approval for an arbitrary process of their own is outside the threat model. The guarantee is that writes require explicit in-app consent and explicit UAC approval, and that the worker's permitted effects remain bounded by the closed catalog.