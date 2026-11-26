# 0004 — Machine Journal and a Trustworthy Hand-Off to the Elevated Worker

**Status:** Accepted

## Context

The UI must stay unprivileged while the worker performs approved Windows service writes and restores. A user-profile journal is writable by the interactive user, caller-supplied path or client identity cannot be trusted just because a binary received UAC approval, and a PID alone does not identify a process over time. The design must retain a catalog-bounded write path without implying protection from arbitrary code that a user explicitly approves through UAC.

## Decision

The machine journal is stored at `C:\ProgramData\Optimi-ez\journal.json`. Before creating its pipe, the worker secures and verifies the journal directory, journal file, and temporary file. If the directory has an untrusted owner, the worker takes ownership as `BUILTIN\Administrators`. The DACL is protected; `SYSTEM` and `BUILTIN\Administrators` receive full control, while `BUILTIN\Users` receive read-only access. Every component in the path is checked for reparse points. A pre-existing `journal.tmp` is rejected if its owner, DACL, or hard-link count is hostile. Failure to establish and verify this state prevents the worker from serving the pipe.

The UI uses a read-only journal interface and cannot append, update, or take an exclusive journal lock. A write requires explicit in-app consent and explicit UAC approval, and the worker accepts only typed operations bounded by the embedded closed catalog. Before restoring service state, the worker also requires the record's action ID, service, target, and snapshot shape to match that catalog. Per-record authentication and its limits are described in ADR-0005.

The worker accepts the one-time pipe name on its command line; the journal path, client PID, and client SID are not supplied through `argv`, and the pipe name itself is not authorization. A peer is admitted only when its image is the UI executable beside the worker and its process creation time predates the worker. The worker records the peer token SID and re-checks the full peer identity before every frame.

One elevated worker serves an app session; it is reused across operations rather than started once per operation. It exits when the app disconnects and also has a ten-minute idle watchdog. It is not a resident service.

Frames larger than the protocol limit are rejected, not truncated. An oversized response is replaced by an explicit `frame_too_large` rejection with the request identifier.

## Alternatives considered

- **Keeping the journal in the user profile and validating records only:** rejected because the unprivileged user can replace or forge its contents between runs.
- **Letting the UI write the journal:** rejected because the unprivileged process must not be able to change the worker's recovery input.
- **Passing journal paths or client identity through arguments:** rejected because a UAC prompt does not display those values; the worker instead uses its compiled machine path and queries the connected peer.
- **Authenticating by PID alone:** rejected because PIDs are recycled; the image, creation time, and token SID are checked together.
- **Truncating an oversized frame or silently closing the pipe:** rejected because either behavior hides the failure and can leave the client waiting.
- **A resident service or a separate worker per operation:** rejected. v0.1 reuses one on-demand worker for an app session and shuts it down on disconnect or idle timeout.

## Consequences

- The journal is readable by `BUILTIN\Users` but writable only by elevated principals. The UI can show recovery status without elevation, but the display-only read is not authorization to write.
- At worker startup, the worker establishes and verifies ownership and DACLs before serving. It does not re-check the directory owner or DACL before every write. If the DACL changes after startup, the running worker does not detect that change at each write.
- Path reparse-point and hard-link checks are not repeated before every runtime journal write; this remains a tracked hardening item. The DACL checker also tolerates extra read-only ACEs; their policy remains to be decided.
- The pipe can receive connections from local users. Admission depends on checking the connected process identity for every frame, not on treating the pipe name as a secret.
- The security guarantee is no write without both in-app consent and explicit UAC approval, with the write constrained to the closed catalog. An attacker who can obtain UAC approval for an arbitrary process of their own is outside the threat model.
- The protocol has an explicit `frame_too_large` rejection. Clients must treat it as a refusal rather than as a successful command.
