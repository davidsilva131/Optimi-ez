# Optimi-ez — Domain Context

Optimi-ez organizes explicit, reversible Windows changes into game-specific profiles. This glossary defines the product vocabulary; it does not describe implementation.

## Profiles and catalog

**Profile**:
A named selection of references to catalog actions for a game or use case. A game profile inherits from a shared base profile; a custom profile can select only actions from the closed catalog.
_Avoid_: preset, tweak pack.

**Base Profile**:
A shared profile named `competitive` or `story` from which a game profile inherits.
_Avoid_: global profile, template (when referring to either named base profile).

**Action**:
A catalog entry describing one permitted Windows change, its risk, conditions, and evidence. An action is not an arbitrary instruction written by the user.
_Avoid_: tweak, command.

**Closed Catalog**:
The reviewed set of actions the application may propose. Profiles may reference catalog actions but cannot define new operations.
_Avoid_: remote catalog, action plugin.

**Windows Identity**:
The Windows account associated with the interactive session in which a person uses the application and gives consent. It is distinct from the elevated identity under which a system operation may run.
_Avoid_: game account, administrator (when referring to the person using the application).

## Review and changes

**Preview**:
A read-only view comparing observed Windows state with a proposed result. It shows applicable actions, no-ops, risks, evidence, and warnings before any change.
_Avoid_: simulated apply, dry run (when it implies that anything has already run).

**Consent**:
An explicit confirmation given after reviewing a preview or restoration request. Without consent, no profile is applied and no requested restoration begins.
_Avoid_: implicit acceptance, auto-apply.

**Snapshot**:
An exact record of the prior values of the fields an action will change, retained to verify and restore that change.
_Avoid_: restore point, approximate backup.

**Apply Transaction**:
A recorded sequence that applies a profile action by action and retains the relevant prior state, Windows build, and verification result for each change. Switching profiles requires verifying restoration of the previous profile first.
_Avoid_: unjournaled batch, automatic profile switch.

**Recovery Required**:
A state in which a transaction cannot be considered verified or restored. It prevents new changes until recovery is resolved.
_Avoid_: partial success, clean state.

**No-op**:
An action that makes no change because its target state already holds or its precondition is not met. It is shown in the preview and is not counted as an applied change.
_Avoid_: applied optimization, empty change.

## Evidence

**Evidence Field**:
Required information for each action and profile that identifies a source, link, and date, or explicitly records that no measurement is available. It does not by itself authorize a performance promise or percentage.
_Avoid_: performance claim, implied benchmark.

**Measurement Pair**:
A comparison of baseline and modified conditions that keeps the machine, Windows build, game, and measurement method constant. Reproducible results are required to support a performance claim.
_Avoid_: anecdote, isolated number.
