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
  "recovery-pending",
  "recovery-writes",
  "recovery-blocker"
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

function recoveryStatus({ pending = [], writesPermitted = false, blocker = null } = {}) {
  return {
    pending_transactions: pending,
    writes_permitted: writesPermitted,
    blocker
  };
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

async function settle() {
  for (let index = 0; index < 6; index += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
}

async function createHarness(handlers) {
  const elements = new Map(
    elementIds.map((id) => [id, new FakeElement("div", id)])
  );
  const staticNodes = ["app.subtitle", "preview.refresh"].map((key) => {
    const element = new FakeElement("span");
    element.dataset.i18n = key;
    return element;
  });
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
    recovery_status: refused({ reason: "write_not_implemented", command: "apply" })
  });
  assert.equal(harness.elements.get("apply").disabled, true);
});

test("a completed apply reports the transaction without claiming verification", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: completed("transaction-9")
  });
  harness.elements.get("apply").click();
  await settle();
  assert.equal(
    harness.elements.get("status").textContent,
    "The change was applied. Transaction transaction-9."
  );
  assert.doesNotMatch(harness.elements.get("status").textContent, /verified/i);
  const applyCall = harness.calls.find((call) => call.command === "apply_preview");
  assert.deepEqual(Object.keys(applyCall.args).sort(), ["consentPreviewId", "previewId"]);
  assert.equal(applyCall.args.previewId, "preview-1");
  assert.equal(applyCall.args.consentPreviewId, "preview-1");
  harness.switchLanguage("es");
  assert.equal(
    harness.elements.get("status").textContent,
    "El cambio se aplicó. Transacción transaction-9."
  );
  assert.doesNotMatch(harness.elements.get("status").textContent, /verificó/i);
  assert.doesNotMatch(harness.elements.get("status").textContent, /falló|failed/i);
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
  assert.equal(harness.elements.get("recovery-pending").textContent, "transaction-7");
  harness.switchLanguage("es");
  assert.equal(
    harness.elements.get("status").textContent,
    "Se requiere recuperación antes de cualquier cambio nuevo. Revisa el panel de recuperación."
  );
  assert.doesNotMatch(harness.elements.get("status").textContent, /falló|failed/i);
});

test("a write refusal is reported as a refusal, not a read failure", async () => {
  const harness = await createHarness({
    preview_profile: completed(previewFixture()),
    recovery_status: completed(recoveryStatus({ writesPermitted: true })),
    apply_preview: refused({ reason: "write_not_implemented", command: "apply" })
  });
  harness.elements.get("apply").click();
  await settle();
  assert.equal(
    harness.elements.get("status").textContent,
    "The write was refused: The apply command exists but writing is not implemented in this build. Nothing was changed."
  );
  harness.switchLanguage("es");
  assert.equal(
    harness.elements.get("status").textContent,
    "La escritura fue rechazada: El comando apply existe, pero la escritura no está implementada en esta compilación. No se cambió nada."
  );
  assert.doesNotMatch(harness.elements.get("status").textContent, /falló|failed/i);
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
    apply_preview: () => inFlight.then(() => completed("transaction-9"))
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
    apply_preview: refused({ reason: "write_not_implemented", command: "apply" })
  });
  assert.equal(harness.elements.get("apply").disabled, false);
  harness.elements.get("apply").click();
  await settle();
  assert.equal(recoveryReads, 2);
  assert.equal(harness.elements.get("apply").disabled, true);
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
