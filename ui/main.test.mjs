import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const mainSource = await readFile(new URL("./main.js", import.meta.url), "utf8");
const i18nSource = await readFile(new URL("./i18n.js", import.meta.url), "utf8");

const elementIds = [
  "windows-version",
  "identity",
  "power-plan",
  "catalog-version",
  "platform-badge",
  "status",
  "action-list",
  "refresh",
  "apply",
  "change-summary",
  "undo",
  "recovery-pending",
  "recovery-writes",
  "recovery-restorable",
  "recovery-blocker",
  "build-change",
  "restore-anyway"
];

class FakeElement {
  constructor(tagName, id = "") {
    this.tagName = tagName;
    this.id = id;
    this.children = [];
    this.listeners = new Map();
    this.attributes = new Map();
    this.dataset = {};
    this.className = "";
    this.textContent = "";
    this.disabled = false;
    const classes = new Set();
    this.classList = {
      add: (name) => classes.add(name),
      remove: (name) => classes.delete(name),
      contains: (name) => classes.has(name),
      toggle: (name, force) => {
        const enabled = force === undefined ? !classes.has(name) : Boolean(force);
        if (enabled) {
          classes.add(name);
        } else {
          classes.delete(name);
        }
        return enabled;
      }
    };
  }

  append(...nodes) {
    this.children.push(...nodes);
  }

  replaceChildren(...nodes) {
    this.children = [...nodes];
  }

  addEventListener(type, listener) {
    this.listeners.set(type, listener);
  }

  setAttribute(name, value) {
    this.attributes.set(name, value);
  }

  click() {
    const listener = this.listeners.get("click");
    if (listener) {
      listener();
    }
  }
}

function completed(payload) {
  return { outcome: "completed", payload };
}

function recoveryRequired(payload) {
  return { outcome: "recovery_required", payload };
}

function refused(payload) {
  return { outcome: "refused", payload };
}

function failed(payload) {
  return { outcome: "failed", payload };
}

function recoveryStatus({ pending = [], writesPermitted = false, blocker = null } = {}) {
  return {
    pending_transactions: pending.map((entry) =>
      typeof entry === "string"
        ? {
            transaction_id: entry,
            service: "DiagTrack",
            state: "recovery_required",
            snapshot: {
              service: "DiagTrack",
              existed: true,
              state: { run_state: "running", start_type: "automatic", delayed_start: false }
            },
            observed: {
              state: "observed",
              value: { run_state: "stopped", start_type: "disabled", delayed_start: false }
            }
          }
        : entry
    ),
    writes_permitted: writesPermitted,
    blocker
  };
}

function pendingTransaction(overrides = {}) {
  return {
    transaction_id: "tx-4",
    service: "DiagTrack",
    state: "recovery_required",
    snapshot: {
      service: "DiagTrack",
      existed: true,
      state: { run_state: "running", start_type: "automatic", delayed_start: true }
    },
    observed: {
      state: "observed",
      value: { run_state: "stopped", start_type: "disabled", delayed_start: false }
    },
    ...overrides
  };
}

function findAll(root, predicate) {
  const found = [];
  if (predicate(root)) {
    found.push(root);
  }
  for (const child of root.children || []) {
    found.push(...findAll(child, predicate));
  }
  return found;
}

function previewFixture() {
  return {
    id: "preview-1",
    profile_id: "competitive",
    catalog_version: "2026.1",
    platform: "supported",
    windows_version: {
      state: "observed",
      value: { major: 10, minor: 0, build: 26100, product_type: "workstation" }
    },
    interactive_identity: { state: "observed", value: { account: "david", domain: null } },
    active_power_plan: { state: "unavailable", failure: { kind: "access_denied" } },
    actions: []
  };
}

function verifiedChange(overrides = {}) {
  return {
    transaction_id: "tx-9",
    action_id: "diagtrack-privacy",
    service: "DiagTrack",
    verified_state: { run_state: "stopped", start_type: "disabled" },
    ...overrides
  };
}

async function settle() {
  for (let index = 0; index < 8; index += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
}

async function createHarness(handlers, options = {}) {
  const elements = new Map(
    elementIds.map((id) => [id, new FakeElement("div", id)])
  );
  const staticNodes = ["app.subtitle", "app.verifiedWrites", "preview.refresh", "change.title"].map(
    (key) => {
      const element = new FakeElement("span");
      element.dataset.i18n = key;
      return element;
    }
  );
  const ariaNodes = ["lang.label"].map((key) => {
    const element = new FakeElement("div");
    element.dataset.i18nAria = key;
    return element;
  });
  const languageButtons = ["en", "es"].map((language) => {
    const element = new FakeElement("button");
    element.dataset.lang = language;
    return element;
  });
  const calls = [];
  const document = {
    documentElement: new FakeElement("html"),
    getElementById: (id) => elements.get(id) || null,
    createElement: (tagName) => new FakeElement(tagName),
    querySelectorAll: (selector) => {
      if (selector === "[data-i18n]") {
        return staticNodes;
      }
      if (selector === "[data-i18n-aria]") {
        return ariaNodes;
      }
      return languageButtons;
    }
  };
  const window = {
    confirm: options.confirm || (() => true),
    __TAURI__: {
      core: {
        invoke: async (command, args) => {
          calls.push({ command, args });
          const handler = handlers[command];
          if (!handler) {
            throw new Error("unexpected command: " + command);
          }
          return typeof handler === "function" ? handler(args) : handler;
        }
      }
    }
  };
  const context = vm.createContext({ window, document, console, setImmediate });
  vm.runInContext(i18nSource, context, { filename: "ui/i18n.js" });
  vm.runInContext(mainSource, context, { filename: "ui/main.js" });
  await settle();
  return {
    elements,
    languageButtons,
    calls,
    switchLanguage: (language) => {
      languageButtons.find((button) => button.dataset.lang === language).click();
    }
  };
}

const readyPreview = () =>
  createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus())
  });

test("apply stays disabled when writes are not permitted", async () => {
  const harness = await readyPreview();
  assert.equal(harness.elements.get("apply").disabled, true);
  assert.equal(harness.elements.get("recovery-writes").textContent, "Not permitted");
  harness.elements.get("apply").click();
  await settle();
  assert.equal(
    harness.calls.filter((call) => call.command === "apply_preview").length,
    0
  );
  assert.equal(harness.elements.get("status").textContent, "Preview generated from the live system.");
});

test("apply is enabled only when writes are permitted with no pending transactions", async () => {
  const permitted = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true }))
  });
  assert.equal(permitted.elements.get("apply").disabled, false);

  const pending = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(
      recoveryStatus({ pending: ["transaction-1"], writesPermitted: true })
    )
  });
  assert.equal(pending.elements.get("apply").disabled, true);
  assert.equal(pending.elements.get("recovery-pending").textContent, "transaction-1");
});

test("apply stays disabled while recovery is unavailable", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: failed("journal_read")
  });
  assert.equal(harness.elements.get("apply").disabled, true);
});

test("a completed apply reports the verified state and offers undo", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: completed(verifiedChange())
  });
  harness.elements.get("apply").click();
  await settle();
  assert.equal(
    harness.elements.get("status").textContent,
    "Applied and verified: DiagTrack is now Stopped / Disabled. Transaction tx-9."
  );
  assert.equal(
    harness.elements.get("change-summary").textContent,
    "DiagTrack is verified at Stopped / Disabled. Transaction tx-9."
  );
  assert.equal(harness.elements.get("undo").disabled, false);
  const applyCall = harness.calls.find((call) => call.command === "apply_preview");
  assert.deepEqual(Object.keys(applyCall.args).sort(), ["consentPreviewId", "previewId"]);
  assert.equal(applyCall.args.previewId, "preview-1");
  assert.equal(applyCall.args.consentPreviewId, "preview-1");
  harness.switchLanguage("es");
  assert.equal(
    harness.elements.get("status").textContent,
    "Aplicado y verificado: DiagTrack ahora está en Detenido / Deshabilitado. Transacción tx-9."
  );
  assert.doesNotMatch(harness.elements.get("status").textContent, /falló|failed/i);
});

test("undo reports the restored state and clears the change", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: completed(verifiedChange()),
    undo_transaction: completed(
      verifiedChange({
        verified_state: { run_state: "running", start_type: "automatic" }
      })
    )
  });
  harness.elements.get("apply").click();
  await settle();
  harness.elements.get("undo").click();
  await settle();
  assert.equal(
    harness.elements.get("status").textContent,
    "Restored and verified: DiagTrack is back to Running / Automatic. Transaction tx-9."
  );
  assert.equal(harness.elements.get("undo").disabled, true);
  assert.equal(
    harness.elements.get("change-summary").textContent,
    "No verified change in this session."
  );
  const undoCall = harness.calls.find((call) => call.command === "undo_transaction");
  assert.equal(undoCall.args.transactionId, "tx-9");
  harness.switchLanguage("es");
  assert.equal(
    harness.elements.get("status").textContent,
    "Restaurado y verificado: DiagTrack volvió a En ejecución / Automático. Transacción tx-9."
  );
});

test("undo stays disabled while another transaction is pending recovery", async () => {
  let reads = 0;
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: () => {
      reads += 1;
      return completed(
        reads === 1
          ? recoveryStatus({ writesPermitted: true })
          : recoveryStatus({
              pending: [pendingTransaction()],
              writesPermitted: false,
              blocker: "pending_recovery"
            })
      );
    },
    apply_preview: completed(verifiedChange())
  });
  assert.equal(harness.elements.get("undo").disabled, true);

  harness.elements.get("apply").click();
  await settle();

  assert.equal(harness.elements.get("apply").disabled, true);
  assert.equal(harness.elements.get("undo").disabled, true);
  harness.elements.get("undo").click();
  await settle();
  assert.equal(
    harness.calls.filter((call) => call.command === "undo_transaction").length,
    0
  );
});

test("recovery_required is its own state in both languages", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: recoveryRequired(
      recoveryStatus({ pending: ["transaction-7"], writesPermitted: false })
    )
  });
  harness.elements.get("apply").click();
  await settle();
  assert.equal(
    harness.elements.get("status").textContent,
    "Recovery is required before any new change. Review the recovery panel."
  );
  assert.equal(harness.elements.get("apply").disabled, true);
  assert.equal(harness.elements.get("undo").disabled, true);
  assert.equal(harness.elements.get("recovery-pending").textContent, "transaction-7");
  harness.switchLanguage("es");
  assert.equal(
    harness.elements.get("status").textContent,
    "Se requiere recuperación antes de cualquier cambio nuevo. Revisa el panel de recuperación."
  );
});

test("the recovery panel restores a pending transaction after a restart", async () => {
  let recovery = recoveryStatus({
    pending: [pendingTransaction()],
    writesPermitted: false,
    blocker: "pending_recovery"
  });
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: () => completed(recovery),
    restore_transaction: (args) => {
      recovery = recoveryStatus({ writesPermitted: true });
      return completed(
        verifiedChange({
          transaction_id: args.transactionId,
          verified_state: {
            run_state: "running",
            start_type: "automatic",
            delayed_start: true
          }
        })
      );
    }
  });

  assert.equal(harness.elements.get("apply").disabled, true);
  assert.equal(harness.elements.get("recovery-pending").textContent, "tx-4");
  const restorable = harness.elements.get("recovery-restorable");
  const buttons = findAll(
    restorable,
    (element) => element.tagName === "button" && element.textContent === "Restore and verify"
  );
  assert.equal(buttons.length, 1);
  assert.equal(buttons[0].textContent, "Restore and verify");
  assert.equal(buttons[0].disabled, false);

  buttons[0].click();
  await settle();

  const restoreCall = harness.calls.find((call) => call.command === "restore_transaction");
  assert.equal(restoreCall.args.transactionId, "tx-4");
  assert.equal(
    harness.elements.get("status").textContent,
    "Restored and verified: DiagTrack is back to Running / Automatic (delayed). Transaction tx-4."
  );
  assert.equal(harness.elements.get("apply").disabled, false);
  assert.equal(harness.elements.get("recovery-pending").textContent, "None");
  assert.equal(findAll(restorable, (element) => element.tagName === "button").length, 0);
});

test("a restore conflict is refused without overwriting the service", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(
      recoveryStatus({ pending: [pendingTransaction()], blocker: "pending_recovery" })
    ),
    restore_transaction: refused({
      reason: "restore_conflict",
      transaction_id: "tx-4",
      expected: { run_state: "running", start_type: "automatic", delayed_start: true },
      current: { run_state: "stopped", start_type: "manual", delayed_start: false }
    })
  });

  const button = findAll(
    harness.elements.get("recovery-restorable"),
    (element) => element.tagName === "button"
  )[0];
  button.click();
  await settle();

  assert.equal(
    harness.elements.get("status").textContent,
    "The write was refused: The service changed outside this transaction: expected Running / Automatic (delayed), found Stopped / Manual. Nothing was restored."
  );
  assert.equal(harness.elements.get("apply").disabled, true);
  harness.switchLanguage("es");
  assert.equal(
    harness.elements.get("status").textContent,
    "La escritura fue rechazada: El servicio cambió fuera de esta transacción: se esperaba En ejecución / Automático (retrasado) y se encontró Detenido / Manual. No se restauró nada."
  );
});

test("a write refusal is reported as a refusal, not a read failure", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: refused({ reason: "preview_not_current", preview_id: "preview-1" })
  });
  harness.elements.get("apply").click();
  await settle();
  assert.equal(
    harness.elements.get("status").textContent,
    "The write was refused: This preview is no longer current. Refresh and review a new preview."
  );
  harness.switchLanguage("es");
  assert.equal(
    harness.elements.get("status").textContent,
    "La escritura fue rechazada: Esta vista previa ya no está vigente. Actualiza y revisa una nueva vista previa."
  );
  assert.doesNotMatch(harness.elements.get("status").textContent, /falló|failed/i);
});

test("a gate refusal names the failed gate", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: refused({ reason: "gate_read_failed", gate: "anti_cheat" })
  });
  harness.elements.get("apply").click();
  await settle();
  assert.equal(
    harness.elements.get("status").textContent,
    "The write was refused: The anti-cheat check could not be read, so the change was blocked. Nothing was changed."
  );
});

test("a command failure is reported as a failure, not verified success", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: failed("journal_write")
  });
  harness.elements.get("apply").click();
  await settle();
  assert.equal(
    harness.elements.get("status").textContent,
    "The command failed: the journal could not be written"
  );
  assert.doesNotMatch(harness.elements.get("status").textContent, /verified/i);
  assert.equal(harness.elements.get("undo").disabled, true);
});

test("an apply bridge error is a write failure, not a read failure", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: () => {
      throw new Error("bridge dropped");
    }
  });
  harness.elements.get("apply").click();
  await settle();
  assert.equal(
    harness.elements.get("status").textContent,
    "The write failed: Error: bridge dropped"
  );
  assert.doesNotMatch(harness.elements.get("status").textContent, /read failed/i);
});

test("a second click cannot start another apply while one is in flight", async () => {
  let release;
  const inFlight = new Promise((resolve) => {
    release = resolve;
  });
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: () => inFlight.then(() => completed(verifiedChange()))
  });
  const apply = harness.elements.get("apply");
  apply.click();
  apply.click();
  assert.equal(apply.disabled, true);
  assert.equal(
    harness.calls.filter((call) => call.command === "apply_preview").length,
    1
  );
  release();
  await settle();
  assert.equal(
    harness.calls.filter((call) => call.command === "apply_preview").length,
    1
  );
});

test("a write refusal re-reads recovery and keeps apply disabled", async () => {
  let recoveryReads = 0;
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: () => {
      recoveryReads += 1;
      return completed(recoveryStatus({ writesPermitted: recoveryReads === 1 }));
    },
    apply_preview: refused({ reason: "preview_not_current", preview_id: "preview-1" })
  });
  assert.equal(harness.elements.get("apply").disabled, false);
  harness.elements.get("apply").click();
  await settle();
  assert.equal(recoveryReads, 2);
  assert.equal(harness.elements.get("apply").disabled, true);
});

test("a build change shows the restore diff and waits for explicit confirmation", async () => {
  const plan = {
    transaction_id: "tx-9",
    service: "DiagTrack",
    recorded_windows: { major: 10, minor: 0, build: 22631, product_type: "workstation" },
    current_windows: { major: 10, minor: 0, build: 26100, product_type: "workstation" },
    snapshot: { run_state: "running", start_type: "automatic", delayed_start: false },
    observed: {
      state: "observed",
      value: { run_state: "stopped", start_type: "disabled", delayed_start: false }
    }
  };
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: completed(verifiedChange()),
    undo_transaction: refused({ reason: "windows_build_changed", plan }),
    restore_after_build_change: completed(
      verifiedChange({ verified_state: { run_state: "running", start_type: "automatic" } })
    )
  });
  harness.elements.get("apply").click();
  await settle();
  assert.equal(harness.elements.get("restore-anyway").disabled, true);

  harness.elements.get("undo").click();
  await settle();

  assert.equal(harness.elements.get("restore-anyway").disabled, false);
  assert.equal(
    harness.elements.get("build-change").textContent,
    "Windows build changed from 22631 to 26100 after this change was recorded. Restoring DiagTrack would set Running / Automatic; it is now Stopped / Disabled. Nothing was restored without confirmation."
  );
  assert.equal(
    harness.elements.get("status").textContent,
    "The write was refused: " + harness.elements.get("build-change").textContent
  );

  harness.elements.get("restore-anyway").click();
  await settle();

  const restoreCall = harness.calls.find(
    (call) => call.command === "restore_after_build_change"
  );
  assert.deepEqual(
    Object.keys(restoreCall.args).sort(),
    ["confirmedBuild", "transactionId"]
  );
  assert.equal(restoreCall.args.transactionId, "tx-9");
  assert.equal(restoreCall.args.confirmedBuild, 26100);
  assert.equal(
    harness.elements.get("status").textContent,
    "Restored and verified: DiagTrack is back to Running / Automatic. Transaction tx-9."
  );
  assert.equal(harness.elements.get("restore-anyway").disabled, true);
  assert.equal(harness.elements.get("build-change").textContent, "");
  harness.switchLanguage("es");
  assert.equal(harness.elements.get("build-change").textContent, "");
});

test("a build change refusal explains the diff in both languages", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: completed(verifiedChange()),
    undo_transaction: refused({
      reason: "windows_build_changed",
      plan: {
        transaction_id: "tx-9",
        service: "DiagTrack",
        recorded_windows: { major: 10, minor: 0, build: 22631, product_type: "workstation" },
        current_windows: { major: 10, minor: 0, build: 26100, product_type: "workstation" },
        snapshot: { run_state: "running", start_type: "automatic", delayed_start: false },
        observed: {
          state: "observed",
          value: { run_state: "stopped", start_type: "disabled", delayed_start: false }
        }
      }
    })
  });
  harness.elements.get("apply").click();
  await settle();
  harness.elements.get("undo").click();
  await settle();
  assert.equal(
    harness.elements.get("status").textContent,
    "The write was refused: Windows build changed from 22631 to 26100 after this change was recorded. Restoring DiagTrack would set Running / Automatic; it is now Stopped / Disabled. Nothing was restored without confirmation."
  );
  harness.switchLanguage("es");
  assert.equal(
    harness.elements.get("status").textContent,
    "La escritura fue rechazada: La compilación de Windows cambió de 22631 a 26100 después de registrar este cambio. Restaurar DiagTrack establecería En ejecución / Automático; ahora está en Detenido / Deshabilitado. No se restauró nada sin confirmación."
  );
});

test("a pending recovery refusal names the blocking transaction", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: completed(verifiedChange()),
    undo_transaction: refused({
      reason: "pending_recovery_blocks_restore",
      transaction_id: "tx-9",
      pending: ["tx-4"]
    })
  });
  harness.elements.get("apply").click();
  await settle();
  harness.elements.get("undo").click();
  await settle();
  assert.equal(
    harness.elements.get("status").textContent,
    "The write was refused: Transaction tx-9 cannot be restored while tx-4 still needs recovery. Nothing was restored."
  );
});

test("abandoning a transaction needs confirmation and is recorded", async () => {
  const recovered = () =>
    recoveryStatus({
      pending: [pendingTransaction()],
      writesPermitted: false,
      blocker: "pending_recovery"
    });
  const handlers = {
    preview_profile: completed(previewFixture()),
    recovery_status: () => completed(recovered()),
    abandon_transaction: (args) => {
      assert.equal(args.transactionId, "tx-4");
      assert.equal(args.service, "DiagTrack");
      return completed({
        transaction_id: "tx-4",
        action_id: "diagtrack-privacy",
        service: "DiagTrack",
        observed: { state: "unavailable", failure: { kind: "not_found" } }
      });
    }
  };

  const declined = await createHarness(handlers, { confirm: () => false });
  const declineCard = declined.elements.get("recovery-restorable").children[0];
  const declineButton = findAll(declineCard, (element) => element.tagName === "button").find(
    (element) => element.textContent === "Abandon"
  );
  assert.ok(declineButton, "an abandon control is offered with the pending transaction");
  declineButton.click();
  await settle();
  assert.equal(
    declined.calls.filter((call) => call.command === "abandon_transaction").length,
    0
  );

  const accepted = await createHarness(handlers);
  const card = accepted.elements.get("recovery-restorable").children[0];
  assert.ok(
    findAll(card, (element) =>
      element.textContent.includes(
        "Abandoning closes this transaction without restoring the service."
      )
    ).length > 0,
    "the abandon control carries its warning"
  );
  const button = findAll(card, (element) => element.tagName === "button").find(
    (element) => element.textContent === "Abandon"
  );
  button.click();
  await settle();
  assert.equal(
    accepted.calls.filter((call) => call.command === "abandon_transaction").length,
    1
  );
  assert.equal(
    accepted.elements.get("status").textContent,
    "Abandoned transaction tx-4. DiagTrack was not restored and the journal records the decision."
  );
});

test("an abandon refusal is reported without closing the transaction", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(
      recoveryStatus({ pending: [pendingTransaction()], blocker: "pending_recovery" })
    ),
    abandon_transaction: refused({
      reason: "transaction_restorable",
      transaction_id: "tx-4"
    })
  });
  const card = harness.elements.get("recovery-restorable").children[0];
  findAll(card, (element) => element.tagName === "button")
    .find((element) => element.textContent === "Abandon")
    .click();
  await settle();
  assert.equal(
    harness.elements.get("status").textContent,
    "The write was refused: Transaction tx-4 can still be restored, so it cannot be abandoned."
  );
  assert.equal(harness.elements.get("recovery-pending").textContent, "tx-4");
});

test("english and spanish dictionaries carry the same keys and placeholders", async () => {
  const context = vm.createContext({ window: {} });
  vm.runInContext(i18nSource, context, { filename: "ui/i18n.js" });
  const { en, es } = context.window.optimiI18n;
  assert.deepEqual(Object.keys(en).sort(), Object.keys(es).sort());
  const placeholders = (text) =>
    [...text.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();
  Object.keys(en).forEach((key) => {
    assert.ok(en[key].length > 0, key);
    assert.ok(es[key].length > 0, key);
    assert.deepEqual(placeholders(en[key]), placeholders(es[key]), key);
  });
});
