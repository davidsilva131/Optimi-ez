# 0002 — Closed Catalog with No Irreversible Actions

**Status:** Accepted

## Context

Service-related Windows changes can affect functions beyond a game and must be reviewable, evidence-carrying, and reversible. An open action system, user scripts, or remote catalog would turn a reviewable product into an executor of operations that have not passed the same safety checks. Available evidence does not justify broad performance claims, and several security and anti-cheat settings are explicitly excluded.

## Decision

Actions are versioned TOML embedded in each release. The catalog is closed: built-in and custom profiles may reference known action IDs, but users cannot create operations. A custom profile may be named, saved, and exported as TOML. Every action and profile carries evidence with a source, URL, and date, or explicitly states that no measurement is available; evidence never becomes a promised FPS or percentage gain.

The v0.1 catalog contains exactly four approved service-related actions: Game DVR background recording, Windows Search Enhanced-to-Classic, Print Spooler only when no printers are present, and DiagTrack as a privacy action only. The evidence for Print Spooler and DiagTrack is not presented as performance evidence. The `power_plan` kind may remain reserved in the type enum, but v0.1 contains no power-plan action and release validation rejects any such entry. The catalog contains no irreversible action; the format may represent an `irreversible` flag, but v0.1 release validation rejects it when true. Only low- or medium-risk actions with bounded evidence are eligible. Generic shell or registry operations, remote catalogs, and unreviewed actions are excluded.

## Alternatives considered

- **User scripts, plugins, or freely defined actions:** rejected because their snapshot, undo, privilege, and anti-cheat behavior cannot be guaranteed.
- **A remotely updated catalog:** rejected because v0.1 is offline and each action change must ship in a reviewed release.
- **Allowing irreversible actions with a warning:** rejected because a warning cannot make an unrecoverable operation reversible.
- **Adding actions to meet a target count:** rejected because catalog size cannot override evidence, reversibility, risk, or the allowlist.
- **Including a power-plan action in v0.1:** rejected because there is no verified evidence to support a latency or FPS improvement and no in-house measurement is in scope.

## Consequences

- The domain validates every action ID and typed operation against the embedded catalog; profile text is never interpreted as a command.
- Custom profiles select existing actions and cannot alter an action's effect.
- Privacy and security rationales remain distinct from performance evidence.
- The four v0.1 action identities are fixed; exact typed Windows fields remain catalog implementation details and cannot expand the release scope.
