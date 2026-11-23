# 0003 — Unprivileged UI and One Domain Command Seam

**Status:** Accepted

## Context

People need to inspect a read-only preview before Windows changes occur, while applying or restoring some changes requires elevation. The product must keep the UI unprivileged, request consent before privileged work, and expose neither arbitrary shell commands nor generic registry operations. A single high-level seam also lets the application logic be tested without making the host machine a test target.

## Decision

The Tauri 2 UI runs as `asInvoker`. The one approved domain command surface is `preview`, `apply(preview_id, consent)`, `undo`, `restore_all`, and `recovery_status`. A typed system-adapter trait sits behind that surface and provides the reads, snapshots, writes, and verification required by catalog actions. The domain behavior is tested through this seam with an in-memory fake adapter; the real Windows adapter is exercised only in a disposable Windows 11 VM.

Privileged writes run in an on-demand elevated worker after explicit consent. The worker accepts only validated domain commands and catalog IDs, re-reads state and gates before each side effect, and never accepts arbitrary commands, registry paths, or executable text. The worker is not resident. UI queries for preview are read-only and do not trigger elevation.

## Alternatives considered

- **Run the whole application elevated:** rejected because it gives the UI renderer unnecessary privileges and prevents a safe, unprivileged preview.
- **Expose separate general-purpose system APIs to the UI:** rejected because they would enlarge the public surface and permit operations outside the approved catalog.
- **Use a resident service or scheduled task:** rejected because v0.1 has no resident process and privileged work must follow explicit consent.
- **Test Windows changes against the development host:** rejected because destructive adapter tests belong in the disposable VM.

## Consequences

- The domain command surface is the primary behavior and test seam; the UI, worker, and Windows adapter do not create alternate paths around it.
- Cancelling elevation, a worker crash, or a missing reply is not success; the journal and a fresh state read determine recovery status.
- The interactive Windows identity that consents is recorded separately from the worker's elevated identity.
- The worker is a platform adapter for the approved seam, not a generic privileged command runner.
