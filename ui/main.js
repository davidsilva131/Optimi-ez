(() => {
  const dictionaries = window.optimiI18n;
  const invoke = window.__TAURI__ && window.__TAURI__.core ? window.__TAURI__.core.invoke : null;
  const profileId = "competitive";

  const state = {
    language: "en",
    preview: null,
    recovery: null,
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
    recoveryPending: document.getElementById("recovery-pending"),
    recoveryWrites: document.getElementById("recovery-writes"),
    recoveryBlocker: document.getElementById("recovery-blocker")
  };

  function t(key, params = {}) {
    const dictionary = dictionaries[state.language] || dictionaries.en;
    const template = dictionary[key] || dictionaries.en[key] || key;
    return template.replace(/\{(\w+)\}/g, (_, name) =>
      params[name] === undefined ? "" : String(params[name])
    );
  }

  function setStatus(key, params = {}) {
    state.status = { key, params };
    renderStatus();
  }

  function showRefusal(refusal) {
    state.status = { refusal };
    renderStatus();
  }

  function renderStatus() {
    if (!elements.status) {
      return;
    }
    if (state.status.refusal) {
      elements.status.textContent = refusalText(state.status.refusal);
    } else if (state.status.key) {
      elements.status.textContent = t(state.status.key, state.status.params);
    } else {
      elements.status.textContent = "";
    }
  }

  function failureText(failure) {
    if (!failure) {
      return t("value.unavailable");
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
    return t("run_state." + value.run_state) + " / " + t("start_type." + value.start_type);
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
    if (refusal.reason === "write_not_implemented") {
      return t("status.writeNotImplemented", { command: refusal.command });
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
    appendFact(facts, t("action.target"), t("start_type." + action.target_start_type));
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
    claim.textContent = action.evidence.claim;
    evidence.append(evidenceTitle, source, url, meta, claim);

    card.append(head, facts, reason, evidence);
    return card;
  }

  function renderPreview() {
    const preview = state.preview;
    elements.platformBadge.className = "badge";
    elements.platformBadge.textContent = "";
    elements.actionList.replaceChildren();
    elements.apply.disabled = true;
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
    elements.apply.disabled = false;
    preview.actions.forEach((action) => elements.actionList.append(renderAction(action)));
  }

  function renderRecovery() {
    const status = state.recovery;
    if (!status) {
      elements.recoveryPending.textContent = "";
      elements.recoveryWrites.textContent = "";
      elements.recoveryBlocker.textContent = "";
      return;
    }
    elements.recoveryPending.textContent = status.pending_transactions.length
      ? status.pending_transactions.join(", ")
      : t("recovery.none");
    elements.recoveryWrites.textContent = status.writes_permitted
      ? t("recovery.permitted")
      : t("recovery.notPermitted");
    elements.recoveryBlocker.textContent = status.blocker
      ? t("recovery.blocker." + status.blocker)
      : "";
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
    renderRecovery();
    renderStatus();
  }

  async function loadPreview() {
    if (!invoke) {
      setStatus("status.bridgeUnavailable");
      return;
    }
    setStatus("status.loading");
    elements.apply.disabled = true;
    try {
      const outcome = await invoke("preview_profile", { profileId });
      if (outcome.outcome === "completed") {
        state.preview = outcome.payload;
        renderPreview();
        setStatus("status.ready");
      } else {
        state.preview = null;
        renderPreview();
        showRefusal(outcome.payload);
      }
    } catch (error) {
      state.preview = null;
      renderPreview();
      setStatus("status.error", { detail: String(error) });
    }
  }

  async function loadRecovery() {
    if (!invoke) {
      return;
    }
    try {
      const outcome = await invoke("recovery_status");
      state.recovery = outcome.outcome === "completed" ? outcome.payload : null;
    } catch (error) {
      state.recovery = null;
    }
    renderRecovery();
  }

  async function applyPreview() {
    if (!invoke || !state.preview) {
      return;
    }
    try {
      const outcome = await invoke("apply_preview", {
        previewId: state.preview.id,
        consentPreviewId: state.preview.id
      });
      if (outcome.outcome === "completed") {
        setStatus("status.applied");
      } else if (outcome.outcome === "refused") {
        showRefusal(outcome.payload);
      } else {
        setStatus("status.error", { detail: outcome.outcome });
      }
    } catch (error) {
      setStatus("status.error", { detail: String(error) });
    }
  }

  elements.refresh.addEventListener("click", loadPreview);
  elements.apply.addEventListener("click", applyPreview);
  document.querySelectorAll("[data-lang]").forEach((button) => {
    button.addEventListener("click", () => setLanguage(button.dataset.lang));
  });

  setLanguage("en");
  loadPreview();
  loadRecovery();
})();
