(() => {
  const dictionaries = window.optimiI18n;
  const invoke = window.__TAURI__ && window.__TAURI__.core ? window.__TAURI__.core.invoke : null;
  const profileId = "competitive";

  const state = {
    language: "en",
    preview: null,
    recovery: null,
    lastChange: null,
    buildChange: null,
    lastAbandon: null,
    applyInFlight: false,
    undoInFlight: false,
    restoreInFlight: false,
    status: { key: "status.loading", params: {} }
  };

  const elements = {
    windowsVersion: document.getElementById("windows-version"),
    identity: document.getElementById("identity"),
    powerPlan: document.getElementById("power-plan"),
    catalogVersion: document.getElementById("catalog-version"),
    platformBadge: document.getElementById("platform-badge"),
    status: document.getElementById("status"),
    actionList: document.getElementById("action-list"),
    refresh: document.getElementById("refresh"),
    apply: document.getElementById("apply"),
    changeSummary: document.getElementById("change-summary"),
    buildChange: document.getElementById("build-change"),
    restoreAnyway: document.getElementById("restore-anyway"),
    undo: document.getElementById("undo"),
    recoveryPending: document.getElementById("recovery-pending"),
    recoveryWrites: document.getElementById("recovery-writes"),
    recoveryRestorable: document.getElementById("recovery-restorable"),
    recoveryBlocker: document.getElementById("recovery-blocker")
  };

  function t(key, params = {}) {
    const dictionary = dictionaries[state.language] || dictionaries.en;
    const template = dictionary[key] || dictionaries.en[key] || key;
    return template.replace(/\{(\w+)\}/g, (_, name) =>
      params[name] === undefined ? "" : String(params[name])
    );
  }

  function rememberBuildChange(refusal) {
    state.buildChange = refusal.reason === "windows_build_changed" ? refusal.plan : null;
    renderApplyAvailability();
  }

  function setStatus(key, params = {}) {
    state.status = { key, params };
    renderStatus();
  }

  function showRefusal(refusal) {
    state.status = { refusal };
    renderStatus();
  }

  function showWriteRefusal(refusal) {
    state.status = { writeRefusal: refusal };
    renderStatus();
  }

  function showWorkerOutcome(outcome) {
    if (outcome.outcome === "elevation_cancelled") {
      setStatus("status.elevationCancelled");
      return true;
    }
    if (outcome.outcome === "worker_unavailable") {
      setStatus("status.workerUnavailable", {
        detail:
          outcome.payload && outcome.payload.detail ? outcome.payload.detail : "unknown"
      });
      return true;
    }
    return false;
  }

  function writesAllowed() {
    if (!state.preview || !state.recovery) {
      return false;
    }
    return state.recovery.writes_permitted && state.recovery.pending_transactions.length === 0;
  }

  function pendingRecovery() {
    return Boolean(
      state.recovery && (state.recovery.pending_transactions || []).length > 0
    );
  }

  function renderApplyAvailability() {
    const busy = state.applyInFlight || state.undoInFlight || state.restoreInFlight;
    elements.apply.disabled = busy || !writesAllowed();
    elements.undo.disabled = busy || !state.lastChange || pendingRecovery();
    renderBuildChange(busy);
  }

  function renderBuildChange(busy) {
    const plan = state.buildChange;
    elements.buildChange.textContent = plan ? buildChangeText(plan) : "";
    elements.restoreAnyway.disabled = busy || !plan;
  }

  function renderStatus() {
    if (!elements.status) {
      return;
    }
    if (state.status.refusal) {
      elements.status.textContent = refusalText(state.status.refusal);
    } else if (state.status.writeRefusal) {
      elements.status.textContent = t("status.writeRefused", {
        detail: refusalText(state.status.writeRefusal)
      });
    } else if (state.status.key) {
      const params = { ...state.status.params };
      if (params.state && typeof params.state === "object") {
        params.state = serviceText(params.state);
      }
      elements.status.textContent = t(state.status.key, params);
    } else {
      elements.status.textContent = "";
    }
  }

  function failureText(failure) {
    if (!failure) {
      return t("value.unavailable");
    }
    if (typeof failure === "string") {
      return t("failure." + failure);
    }
    if (failure.kind === "system_error") {
      return t("failure.system_error", { code: failure.code });
    }
    return t("failure." + failure.kind);
  }

  function observationText(observation, formatValue) {
    if (!observation) {
      return t("value.unavailable");
    }
    if (observation.state === "observed" && observation.value) {
      return formatValue(observation.value);
    }
    return t("value.unavailable") + " (" + failureText(observation.failure) + ")";
  }

  function windowsText(value) {
    return t("system.windowsValue", value);
  }

  function identityText(value) {
    return value.domain ? value.domain + "\\" + value.account : value.account;
  }

  function powerPlanText(value) {
    return value.name ? value.name + " (" + value.guid + ")" : value.guid;
  }

  function serviceText(value) {
    const base = t("run_state." + value.run_state) + " / " + t("start_type." + value.start_type);
    return value.delayed_start ? base + " (" + t("service.delayed") + ")" : base;
  }

  function buildChangeText(plan) {
    return t("status.windowsBuildChanged", {
      service: plan.service,
      recordedBuild: plan.recorded_windows.build,
      currentBuild: plan.current_windows.build,
      snapshot: serviceText(plan.snapshot),
      current: observationText(plan.observed, serviceText)
    });
  }

  function refusalText(refusal) {
    if (!refusal) {
      return "";
    }
    if (refusal.reason === "unknown_profile") {
      return t("status.unknownProfile", { profileId: refusal.profile_id });
    }
    if (refusal.reason === "invalid_preview_id") {
      return t("status.invalidPreview", { value: refusal.value });
    }
    if (refusal.reason === "invalid_transaction_id") {
      return t("status.invalidTransaction", { value: refusal.value });
    }
    if (refusal.reason === "consent_mismatch") {
      return t("status.consentMismatch");
    }
    if (refusal.reason === "preview_not_current") {
      return t("status.previewNotCurrent");
    }
    if (refusal.reason === "preview_state_changed") {
      return t("status.previewStateChanged");
    }
    if (refusal.reason === "preview_state_unavailable") {
      return t("status.previewStateUnavailable");
    }
    if (refusal.reason === "no_applicable_action") {
      return t("status.noApplicableAction");
    }
    if (refusal.reason === "batch_apply_unsupported") {
      return t("status.batchApplyUnsupported", { count: refusal.actions });
    }
    if (refusal.reason === "service_state_not_settled") {
      return t("status.serviceStateNotSettled", { state: t("run_state." + refusal.state) });
    }
    if (refusal.reason === "gate_blocked") {
      return t("status.gateBlocked", { gate: t("gate." + refusal.gate) });
    }
    if (refusal.reason === "gate_read_failed") {
      return t("status.gateReadFailed", { gate: t("gate." + refusal.gate) });
    }
    if (refusal.reason === "write_in_progress") {
      return t("status.writeInProgress");
    }
    if (refusal.reason === "unknown_transaction") {
      return t("status.unknownTransaction", { transactionId: refusal.transaction_id });
    }
    if (refusal.reason === "journal_record_untrusted") {
      return t("status.journalRecordUntrusted", { transactionId: refusal.transaction_id });
    }
    if (refusal.reason === "transaction_not_restorable") {
      return t("status.transactionNotRestorable", {
        transactionId: refusal.transaction_id
      });
    }
    if (refusal.reason === "already_restored") {
      return t("status.alreadyRestored", { transactionId: refusal.transaction_id });
    }
    if (refusal.reason === "restore_conflict") {
      return t("status.restoreConflict", {
        expected: serviceText(refusal.expected),
        current: serviceText(refusal.current)
      });
    }
    if (refusal.reason === "restore_state_unavailable") {
      return t("status.restoreStateUnavailable", {
        transactionId: refusal.transaction_id
      });
    }
    if (refusal.reason === "windows_build_changed") {
      return buildChangeText(refusal.plan);
    }
    if (refusal.reason === "pending_recovery_blocks_restore") {
      return t("status.pendingRecoveryBlocksRestore", {
        transactionId: refusal.transaction_id,
        pending: (refusal.pending || []).join(", ")
      });
    }
    if (refusal.reason === "transaction_abandoned") {
      return t("status.transactionAbandoned", {
        transactionId: refusal.transaction_id
      });
    }
    if (refusal.reason === "already_abandoned") {
      return t("status.alreadyAbandoned", {
        transactionId: refusal.transaction_id
      });
    }
    if (refusal.reason === "transaction_restorable") {
      return t("status.transactionRestorable", {
        transactionId: refusal.transaction_id
      });
    }
    if (refusal.reason === "abandon_consent_mismatch") {
      return t("status.abandonConsentMismatch");
    }
    return t("status.error", { detail: refusal.reason });
  }

  function appendFact(list, term, value) {
    const row = document.createElement("div");
    const termElement = document.createElement("dt");
    termElement.textContent = term;
    const valueElement = document.createElement("dd");
    valueElement.textContent = value;
    row.append(termElement, valueElement);
    list.append(row);
  }

  function renderAction(action) {
    const card = document.createElement("article");
    card.className = "card";

    const head = document.createElement("header");
    const titleGroup = document.createElement("div");
    const eyebrow = document.createElement("span");
    eyebrow.className = "eyebrow";
    eyebrow.textContent = action.action_id;
    const title = document.createElement("h3");
    title.textContent = action.service;
    titleGroup.append(eyebrow, title);
    const badge = document.createElement("span");
    badge.className = "badge disposition-" + action.disposition;
    badge.textContent = t("disposition." + action.disposition);
    head.append(titleGroup, badge);

    const facts = document.createElement("dl");
    facts.className = "grid compact";
    appendFact(facts, t("action.observed"), observationText(action.observed, serviceText));
    appendFact(facts, t("action.target"), serviceText(action.target));
    appendFact(facts, t("action.risk"), t("risk." + action.risk));

    const reason = document.createElement("p");
    reason.className = "reason";
    reason.textContent = t("reason." + action.reason);

    const evidence = document.createElement("div");
    evidence.className = "evidence";
    const evidenceTitle = document.createElement("p");
    evidenceTitle.className = "evidence-title";
    evidenceTitle.textContent = t("action.evidence");
    const source = document.createElement("p");
    source.className = "evidence-source";
    source.textContent = action.evidence.source;
    const url = document.createElement("code");
    url.className = "evidence-url";
    url.textContent = action.evidence.url;
    const meta = document.createElement("p");
    meta.className = "evidence-meta";
    meta.textContent = t("action.evidenceLine", {
      date: action.evidence.date,
      measurement: t("measurement." + action.evidence.measurement)
    });
    const claim = document.createElement("p");
    claim.className = "evidence-claim";
    claim.textContent = t(action.evidence.claim_key);
    evidence.append(evidenceTitle, source, url, meta, claim);

    card.append(head, facts, reason, evidence);
    return card;
  }

  function renderPreview() {
    const preview = state.preview;
    elements.platformBadge.className = "badge";
    elements.platformBadge.textContent = "";
    elements.actionList.replaceChildren();
    renderApplyAvailability();
    elements.windowsVersion.textContent = "";
    elements.identity.textContent = "";
    elements.powerPlan.textContent = "";
    elements.catalogVersion.textContent = "";

    if (!preview) {
      return;
    }

    elements.windowsVersion.textContent = observationText(preview.windows_version, windowsText);
    elements.identity.textContent = observationText(preview.interactive_identity, identityText);
    elements.powerPlan.textContent = observationText(preview.active_power_plan, powerPlanText);
    elements.catalogVersion.textContent = preview.catalog_version;
    elements.platformBadge.textContent = t("platform." + preview.platform);
    elements.platformBadge.classList.add("platform-" + preview.platform);
    preview.actions.forEach((action) => elements.actionList.append(renderAction(action)));
  }

  function renderChange() {
    const change = state.lastChange;
    if (!change) {
      elements.changeSummary.textContent = t("change.none");
      renderApplyAvailability();
      return;
    }
    elements.changeSummary.textContent = t("change.summary", {
      service: change.service,
      state: serviceText(change.verified_state),
      transactionId: change.transaction_id
    });
    renderApplyAvailability();
  }

  function snapshotText(snapshot) {
    if (!snapshot || !snapshot.state) {
      return t("value.unavailable");
    }
    return serviceText(snapshot.state);
  }

  function renderRestorable(transaction) {
    const card = document.createElement("article");
    card.className = "card recovery-card";

    const head = document.createElement("header");
    const titleGroup = document.createElement("div");
    const eyebrow = document.createElement("span");
    eyebrow.className = "eyebrow";
    eyebrow.textContent = transaction.transaction_id;
    const title = document.createElement("h3");
    title.textContent = transaction.service;
    titleGroup.append(eyebrow, title);
    head.append(titleGroup);

    const facts = document.createElement("dl");
    facts.className = "grid compact";
    appendFact(facts, t("recovery.state"), t("transaction_state." + transaction.state));
    appendFact(facts, t("recovery.snapshot"), snapshotText(transaction.snapshot));
    appendFact(facts, t("recovery.observed"), observationText(transaction.observed, serviceText));

    const button = document.createElement("button");
    button.type = "button";
    button.className = "button";
    button.textContent = t("recovery.restore");
    button.disabled = state.applyInFlight || state.undoInFlight || state.restoreInFlight;
    button.addEventListener("click", () => restoreTransaction(transaction.transaction_id));

    const abandon = document.createElement("button");
    abandon.type = "button";
    abandon.className = "button";
    abandon.textContent = t("recovery.abandon");
    abandon.disabled = state.applyInFlight || state.undoInFlight || state.restoreInFlight;
    abandon.addEventListener("click", () => abandonTransaction(transaction));

    const warning = document.createElement("p");
    warning.className = "hint";
    warning.textContent = t("recovery.abandonWarning");

    card.append(head, facts, button, abandon, warning);
    return card;
  }

  function renderAbandoned(abandoned) {
    const card = document.createElement("article");
    card.className = "card recovery-card";

    const head = document.createElement("header");
    const titleGroup = document.createElement("div");
    const eyebrow = document.createElement("span");
    eyebrow.className = "eyebrow";
    eyebrow.textContent = abandoned.transaction_id;
    const title = document.createElement("h3");
    title.textContent = abandoned.service;
    titleGroup.append(eyebrow, title);
    head.append(titleGroup);

    const facts = document.createElement("dl");
    facts.className = "grid compact";
    appendFact(facts, t("recovery.state"), t("transaction_state.abandoned"));
    appendFact(facts, t("recovery.observed"), observationText(abandoned.observed, serviceText));

    card.append(head, facts);
    return card;
  }

  function renderRecovery() {
    const status = state.recovery;
    elements.recoveryRestorable.replaceChildren();
    if (!status) {
      elements.recoveryPending.textContent = "";
      elements.recoveryWrites.textContent = "";
      elements.recoveryBlocker.textContent = "";
      renderApplyAvailability();
      return;
    }
    const pending = status.pending_transactions || [];
    elements.recoveryPending.textContent = pending.length
      ? pending.map((transaction) => transaction.transaction_id).join(", ")
      : t("recovery.none");
    elements.recoveryWrites.textContent = status.writes_permitted
      ? t("recovery.permitted")
      : t("recovery.notPermitted");
    elements.recoveryBlocker.textContent = status.blocker
      ? t("recovery.blocker." + status.blocker)
      : "";
    pending.forEach((transaction) =>
      elements.recoveryRestorable.append(renderRestorable(transaction))
    );
    if (state.lastAbandon) {
      elements.recoveryRestorable.append(renderAbandoned(state.lastAbandon));
    }
    renderApplyAvailability();
  }

  function applyStaticText() {
    document.documentElement.lang = state.language;
    document.querySelectorAll("[data-i18n]").forEach((element) => {
      element.textContent = t(element.dataset.i18n);
    });
    document.querySelectorAll("[data-i18n-aria]").forEach((element) => {
      element.setAttribute("aria-label", t(element.dataset.i18nAria));
    });
  }

  function setLanguage(language) {
    state.language = language;
    document.querySelectorAll("[data-lang]").forEach((button) => {
      button.classList.toggle("active", button.dataset.lang === language);
    });
    applyStaticText();
    renderPreview();
    renderChange();
    renderRecovery();
    renderStatus();
  }

  async function loadPreview(options = {}) {
    const quiet = options.quiet === true;
    if (!invoke) {
      setStatus("status.bridgeUnavailable");
      return;
    }
    if (!quiet) {
      setStatus("status.loading");
      elements.apply.disabled = true;
    }
    try {
      const outcome = await invoke("preview_profile", { profileId });
      if (outcome.outcome === "completed") {
        state.preview = outcome.payload;
        renderPreview();
        if (!quiet) {
          setStatus("status.ready");
        }
      } else if (outcome.outcome === "recovery_required") {
        state.preview = null;
        state.recovery = outcome.payload;
        renderPreview();
        renderRecovery();
        setStatus("status.recoveryRequired");
      } else if (outcome.outcome === "refused") {
        state.preview = null;
        renderPreview();
        if (!quiet) {
          showRefusal(outcome.payload);
        }
      } else {
        state.preview = null;
        renderPreview();
        if (!quiet) {
          setStatus("status.unexpectedOutcome", { detail: outcome.outcome });
        }
      }
    } catch (error) {
      state.preview = null;
      renderPreview();
      if (!quiet) {
        setStatus("status.error", { detail: String(error) });
      }
    }
  }

  async function loadRecovery() {
    if (!invoke) {
      return;
    }
    try {
      const outcome = await invoke("recovery_status");
      state.recovery =
        outcome.outcome === "completed" || outcome.outcome === "recovery_required"
          ? outcome.payload
          : null;
    } catch (error) {
      state.recovery = null;
    }
    renderRecovery();
  }

  async function applyPreview() {
    if (
      !invoke ||
      !state.preview ||
      state.applyInFlight ||
      state.undoInFlight ||
      state.restoreInFlight ||
      !writesAllowed()
    ) {
      return;
    }
    state.applyInFlight = true;
    renderApplyAvailability();
    const previewId = state.preview.id;
    let reloadRecovery = true;
    try {
      const outcome = await invoke("apply_preview", {
        previewId,
        consentPreviewId: previewId
      });
      if (outcome.outcome === "completed") {
        state.buildChange = null;
        state.lastChange = outcome.payload;
        renderChange();
        setStatus("status.applyVerified", {
          service: outcome.payload.service,
          state: outcome.payload.verified_state,
          transactionId: outcome.payload.transaction_id
        });
        await loadPreview({ quiet: true });
      } else if (outcome.outcome === "recovery_required") {
        state.lastChange = null;
        state.recovery = outcome.payload;
        reloadRecovery = false;
        renderChange();
        renderRecovery();
        setStatus("status.recoveryRequired");
      } else if (outcome.outcome === "refused") {
        rememberBuildChange(outcome.payload);
        showWriteRefusal(outcome.payload);
      } else if (outcome.outcome === "failed") {
        setStatus("status.commandFailed", { detail: failureText(outcome.payload) });
      } else if (!showWorkerOutcome(outcome)) {
        setStatus("status.unexpectedOutcome", { detail: outcome.outcome });
      }
    } catch (error) {
      setStatus("status.writeFailed", { detail: String(error) });
    } finally {
      if (reloadRecovery) {
        await loadRecovery();
      }
      state.applyInFlight = false;
      renderApplyAvailability();
    }
  }

  async function undoChange() {
    if (
      !invoke ||
      !state.lastChange ||
      pendingRecovery() ||
      state.applyInFlight ||
      state.undoInFlight ||
      state.restoreInFlight
    ) {
      return;
    }
    state.undoInFlight = true;
    renderApplyAvailability();
    const transactionId = state.lastChange.transaction_id;
    let reloadRecovery = true;
    try {
      const outcome = await invoke("undo_transaction", { transactionId });
      if (outcome.outcome === "completed") {
        state.buildChange = null;
        state.lastChange = null;
        renderChange();
        setStatus("status.undoVerified", {
          service: outcome.payload.service,
          state: outcome.payload.verified_state,
          transactionId: outcome.payload.transaction_id
        });
        await loadPreview({ quiet: true });
      } else if (outcome.outcome === "recovery_required") {
        state.lastChange = null;
        state.recovery = outcome.payload;
        reloadRecovery = false;
        renderChange();
        renderRecovery();
        setStatus("status.recoveryRequired");
      } else if (outcome.outcome === "refused") {
        rememberBuildChange(outcome.payload);
        showWriteRefusal(outcome.payload);
      } else if (outcome.outcome === "failed") {
        setStatus("status.commandFailed", { detail: failureText(outcome.payload) });
      } else if (!showWorkerOutcome(outcome)) {
        setStatus("status.unexpectedOutcome", { detail: outcome.outcome });
      }
    } catch (error) {
      setStatus("status.writeFailed", { detail: String(error) });
    } finally {
      if (reloadRecovery) {
        await loadRecovery();
      }
      state.undoInFlight = false;
      renderApplyAvailability();
    }
  }

  async function restoreTransaction(transactionId) {
    if (
      !invoke ||
      !transactionId ||
      state.applyInFlight ||
      state.undoInFlight ||
      state.restoreInFlight
    ) {
      return;
    }
    state.restoreInFlight = true;
    renderRecovery();
    let reloadRecovery = true;
    try {
      const outcome = await invoke("restore_transaction", { transactionId });
      if (outcome.outcome === "completed") {
        state.buildChange = null;
        setStatus("status.undoVerified", {
          service: outcome.payload.service,
          state: outcome.payload.verified_state,
          transactionId: outcome.payload.transaction_id
        });
        await loadPreview({ quiet: true });
      } else if (outcome.outcome === "recovery_required") {
        state.recovery = outcome.payload;
        reloadRecovery = false;
        renderRecovery();
        setStatus("status.recoveryRequired");
      } else if (outcome.outcome === "refused") {
        rememberBuildChange(outcome.payload);
        showWriteRefusal(outcome.payload);
      } else if (outcome.outcome === "failed") {
        setStatus("status.commandFailed", { detail: failureText(outcome.payload) });
      } else if (!showWorkerOutcome(outcome)) {
        setStatus("status.unexpectedOutcome", { detail: outcome.outcome });
      }
    } catch (error) {
      setStatus("status.writeFailed", { detail: String(error) });
    } finally {
      state.restoreInFlight = false;
      if (reloadRecovery) {
        await loadRecovery();
      }
      renderApplyAvailability();
    }
  }

  async function restoreAnyway() {
    const plan = state.buildChange;
    if (
      !invoke ||
      !plan ||
      state.applyInFlight ||
      state.undoInFlight ||
      state.restoreInFlight
    ) {
      return;
    }
    state.restoreInFlight = true;
    renderApplyAvailability();
    let reloadRecovery = true;
    try {
      const outcome = await invoke("restore_after_build_change", {
        transactionId: plan.transaction_id,
        confirmedBuild: plan.current_windows.build
      });
      if (outcome.outcome === "completed") {
        state.buildChange = null;
        if (state.lastChange && state.lastChange.transaction_id === plan.transaction_id) {
          state.lastChange = null;
          renderChange();
        }
        setStatus("status.undoVerified", {
          service: outcome.payload.service,
          state: outcome.payload.verified_state,
          transactionId: outcome.payload.transaction_id
        });
        await loadPreview({ quiet: true });
      } else if (outcome.outcome === "recovery_required") {
        state.recovery = outcome.payload;
        reloadRecovery = false;
        renderRecovery();
        setStatus("status.recoveryRequired");
      } else if (outcome.outcome === "refused") {
        rememberBuildChange(outcome.payload);
        showWriteRefusal(outcome.payload);
      } else if (outcome.outcome === "failed") {
        setStatus("status.commandFailed", { detail: failureText(outcome.payload) });
      } else if (!showWorkerOutcome(outcome)) {
        setStatus("status.unexpectedOutcome", { detail: outcome.outcome });
      }
    } catch (error) {
      setStatus("status.writeFailed", { detail: String(error) });
    } finally {
      state.restoreInFlight = false;
      if (reloadRecovery) {
        await loadRecovery();
      }
      renderApplyAvailability();
    }
  }

  async function abandonTransaction(transaction) {
    if (
      !invoke ||
      !transaction ||
      state.applyInFlight ||
      state.undoInFlight ||
      state.restoreInFlight
    ) {
      return;
    }
    if (typeof window.confirm === "function" && !window.confirm(t("recovery.abandonWarning"))) {
      return;
    }
    state.restoreInFlight = true;
    renderRecovery();
    let reloadRecovery = true;
    try {
      const outcome = await invoke("abandon_transaction", {
        transactionId: transaction.transaction_id,
        service: transaction.service
      });
      if (outcome.outcome === "completed") {
        state.buildChange = null;
        state.lastAbandon = outcome.payload;
        setStatus("status.abandoned", {
          transactionId: outcome.payload.transaction_id,
          service: outcome.payload.service
        });
        await loadPreview({ quiet: true });
      } else if (outcome.outcome === "recovery_required") {
        state.recovery = outcome.payload;
        reloadRecovery = false;
        renderRecovery();
        setStatus("status.recoveryRequired");
      } else if (outcome.outcome === "refused") {
        showWriteRefusal(outcome.payload);
      } else if (outcome.outcome === "failed") {
        setStatus("status.commandFailed", { detail: failureText(outcome.payload) });
      } else if (!showWorkerOutcome(outcome)) {
        setStatus("status.unexpectedOutcome", { detail: outcome.outcome });
      }
    } catch (error) {
      setStatus("status.writeFailed", { detail: String(error) });
    } finally {
      state.restoreInFlight = false;
      if (reloadRecovery) {
        await loadRecovery();
      }
      renderApplyAvailability();
    }
  }

  elements.refresh.addEventListener("click", () => loadPreview());
  elements.apply.addEventListener("click", applyPreview);
  elements.undo.addEventListener("click", undoChange);
  elements.restoreAnyway.addEventListener("click", restoreAnyway);
  document.querySelectorAll("[data-lang]").forEach((button) => {
    button.addEventListener("click", () => setLanguage(button.dataset.lang));
  });

  setLanguage("en");
  loadPreview();
  loadRecovery();
})();
