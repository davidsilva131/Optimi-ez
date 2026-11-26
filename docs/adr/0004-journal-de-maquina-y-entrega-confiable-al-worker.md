# 0004 — Machine Journal and a Trustworthy Hand-Off to the Elevated Worker

**Status:** Accepted

## Context

The elevated worker performs Windows writes that the unprivileged UI cannot perform, so anything the worker reads must be as trustworthy as the worker itself. Two surfaces were weaker than that:

- The journal lived under `%APPDATA%` of the interactive user. That user could write it, and restoring a transaction rewrote the service named by the record. A forged `applying` record therefore turned "Restore" into "disable this service", without a second elevation prompt and without touching the pipe.
- The worker took its journal path, the consenting SID, and the expected client PID from its own command line. Any process of the same user could launch the worker through `runas`, approve a prompt that shows the executable but not the arguments, and obtain an elevated JSON writer for an arbitrary path plus a channel whose authorized identity the attacker chose.

A third weakness was identity by PID alone: named pipes are enumerable by the same user, and a recycled PID could satisfy a PID-only comparison.

## Decision

The journal is machine state, not user state. It lives at `%PROGRAMDATA%\Optimi-ez\journal.json` with a protected DACL that grants full control to `SYSTEM` and `Administrators` and read-only access to `BUILTIN\Users`. The elevated worker creates the directory, applies that DACL, and verifies the resulting DACL before it creates its pipe; if the verification fails it exits without serving. The unprivileged process opens the journal through a read-only journal type, so its journal handle refuses every append, update, and exclusive lock.

Restoration trusts nothing it did not write. Before a record can write service state, the record must match the embedded catalog: the action ID exists, the service and the target equal that action's service and target, and the snapshot has a restorable shape with the same service name. A record that does not match is refused with `journal_record_untrusted`, and nothing is written.

The worker's command line carries exactly one value: the one-time pipe name. The journal path is compiled into the worker, and the client's identity never travels through the command line. The worker derives the consenting identity from the token of the process connected to the pipe and admits a client only when its image is the UI executable installed next to the worker and its creation time predates the worker's own. The admitting identity is pinned for the connection and re-checked before each frame, so a reused PID with a different creation time, a different token SID, or a different image is rejected.

Frames that cannot be represented within the protocol limit are reported, never truncated. A response that exceeds the frame budget is replaced by an explicit `frame_too_large` rejection carrying the request identifier, so the client fails immediately instead of waiting for the idle watchdog.

## Alternatives considered

- **Keeping the journal in the user profile and validating records only:** rejected because a user-writable file is user-controlled input, and read-only validation still leaves the journal replaceable, deletable, and forgeable between runs.
- **Letting the UI write the journal and the worker read it:** rejected because the writer would then be the unprivileged process the journal exists to constrain.
- **Passing the journal path, consenting SID, and client PID as arguments:** rejected because a UAC prompt shows the executable, not the arguments, so any process of the same user can choose them.
- **Authenticating the client by PID alone:** rejected because PIDs are recycled; creation time and image path make the comparison stable.
- **Truncating an oversized response or closing the pipe silently:** rejected because a silent close is indistinguishable from a crash and leaves the client blocked until the watchdog fires.
- **A resident service or a broker process:** out of scope for v0.1, which keeps one on-demand worker per consented operation.

## Consequences

- Journal contents are readable by any local user and writable only by elevated processes; `recovery_status` keeps working without elevation.
- The worker verifies the directory and file DACL before it serves and refuses to create its pipe when the journal is writable outside Administrators and SYSTEM. Directory ownership is not re-verified, so a directory squatted by another user can still deny service, but it cannot inject a record that survives catalog corroboration.
- Every journal record that reaches a write is corroborated against the catalog, so a tampered journal can only produce refusals, not service changes.
- The pipe accepts connection attempts from any local user, but only the process that launched the worker is served; the confidentiality of the one-time pipe name, unreadable to unprivileged processes, is the remaining secret.
- A worker launched outside the UI cannot name another client, another journal, or another identity.
- The protocol grows one explicit rejection path instead of a silent empty frame; clients must treat `frame_too_large` as a refused command, not as a successful one.
