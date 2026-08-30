// Settings: Providers, Profiles, General. Every action is a core operation through the bridge;
// the surface composes and never decides. Destructive actions confirm in a page sheet, because
// WKWebView does not implement window.confirm and the styling is ours anyway.

import { call, esc, tile, tools, cap, plural } from "./ui.js";

const surface = document.getElementById("surface");

// Esc closes Settings like every other surface. The in-page sheet dialog swallows its own
// Esc first (stopPropagation in its handler) so closing a dialog never closes the window
// under it.
const { getCurrentWindow } = window.__TAURI__.window;
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") getCurrentWindow().hide();
});

let tab = "providers";

export function settings() {
  surface.className = "settings";
  draw();
}

async function draw() {
  surface.innerHTML = `
    <nav class="tabs">
      <button data-tab="providers">Providers</button>
      <button data-tab="profiles">Profiles</button>
      <button data-tab="general">General</button>
    </nav>
    <section id="tab"></section>`;
  surface.querySelectorAll(".tabs button").forEach((b) =>
    b.addEventListener("click", () => {
      tab = b.dataset.tab;
      surface.querySelectorAll(".tabs button").forEach((x) => x.classList.toggle("active", x === b));
      draw();
    })
  );
  surface.querySelector('[data-tab="providers"]').classList.add("active");
  const pane = document.getElementById("tab");
  if (tab === "providers") await providers(pane);
  else if (tab === "profiles") await profilesTab(pane);
  else general(pane);
}

// -- Providers -------------------------------------------------------------------------

async function providers(pane) {
  const [{ providers }, toolList] = await Promise.all([
    call({ version: 1, op: "list_providers", params: {} }),
    tools(),
  ]);
  pane.innerHTML =
    providers
      .map((p) => {
        const format = !p.formats
          ? "Unknown until you test"
          : p.formats.length === 0
            ? "Serves none of your tools"
            : p.formats.length === toolList.length
              ? `Serves all ${p.formats.length} tools`
              : `Serves ${p.formats.join(", ")}`;
        // ADR-0007: the credential line is per tool, not per app — and for OpenCode the key is
        // on disk. A provider serving the chat format reaches OpenCode, so the row must say so
        // instead of the Keychain reassurance.
        const reachesOpenCode = p.formats?.some((f) => f === "openai_chat") ?? true;
        const keyLine = reachesOpenCode
          ? "OpenCode has no key helper — written to a file only you can read"
          : "Claude Code and Codex fetch it through a helper command";
        return `
      <div class="card" data-id="${esc(p.id)}">
        <div class="row">
          ${tile(p.name)}
          <span class="label">${esc(p.name)}</span>
          <span class="value">${esc(p.enabled ? "" : "off")}</span>
        </div>
        <div class="row"><span class="label">Endpoint</span><span class="value">${esc(p.base_url)}</span></div>
        <div class="row"><span class="label">API format</span><span class="value">${esc(format)}</span></div>
        <div class="row"><span class="note">${esc(keyLine)}</span></div>
        <div class="actions"><button class="act" data-do="test">Test</button>
          <button class="act" data-do="toggle">${p.enabled ? "Turn off" : "Turn on"}</button>
          <button class="act danger" data-do="remove">Remove provider…</button></div>
        <div class="result note" role="status"></div>
      </div>`;
      })
      .join("") +
    `
    <div class="card">
      <div class="field"><span>Name</span><input id="np-name" placeholder="e.g. Work OpenRouter" /></div>
      <div class="field"><span>Base URL</span><input id="np-url" placeholder="https://api.example.com/v1" /></div>
      <div class="field"><span>API key</span><input id="np-key" type="password" placeholder="Paste a key" /></div>
      <label class="field"><input type="checkbox" id="np-auto" checked /> Also create a profile</label>
      <p class="note">Switchable right away</p>
      <button class="act" id="np-add">Add provider</button>
      <div class="result note" role="status"></div>
    </div>`;

  const refused = (result, r) => {
    result.textContent = r.failure ? `${r.failure.kind} — nothing was changed` : "";
  };

  pane.querySelectorAll(".card .actions").forEach((actions) => {
    const card = actions.closest(".card");
    const id = card.dataset.id;
    const result = card.querySelector(".result");
    actions.querySelectorAll("button").forEach((button) =>
      button.addEventListener("click", async () => {
        result.textContent = "";
        const doing = button.dataset.do;
        if (doing === "test") {
          button.disabled = true;
          const started = performance.now();
          const r = await call({ version: 1, op: "test", params: { provider_id: id } });
          button.disabled = false;
          result.textContent = !r.knowable
            ? "This endpoint answers everything — its formats can’t be read from here"
            : r.formats.some((f) => f.served)
              ? `${Math.round(performance.now() - started)} ms · answers ${r.formats.filter((f) => f.served).map((f) => f.format).join(", ")}`
              : "Serves none of your tools";
        } else if (doing === "toggle") {
          const p = providers.find((x) => x.id === id);
          const r = await call({ version: 1, op: "set_provider_enabled", params: { id, enabled: !p.enabled } });
          if (r.ok) draw();
          else refused(result, r);
        } else if (doing === "remove") {
          confirmSheet(
            "Remove provider…",
            `Its entries go from ${toolList.map(cap).join(", ")}; the key tapkey stored is deleted`,
            async () => {
              const r = await call({ version: 1, op: "remove_provider", params: { id } });
              if (r.ok) draw();
              else refused(result, r);
            }
          );
        }
      })
    );
  });

  const form = pane.querySelectorAll(".card")[pane.querySelectorAll(".card").length - 1];
  const formResult = form.querySelector(".result");
  form.querySelector("#np-add").addEventListener("click", async () => {
    const name = form.querySelector("#np-name").value.trim();
    const base = form.querySelector("#np-url").value.trim();
    const key = form.querySelector("#np-key").value.trim();
    const id = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || `provider-${Date.now()}`;
    const created = await call({
      version: 1, op: "create_provider",
      params: { id, name: name || id, base_url: base },
    });
    if (!created.ok) return refused(formResult, created);
    if (key) {
      const stored = await call({
        version: 1, op: "set_credential", params: { provider_id: id, secret: key },
      });
      if (!stored.ok) return refused(formResult, stored);
    }
    if (form.querySelector("#np-auto").checked) {
      // Two calls joined by the interface, per ticket 31: a provider without a profile is an
      // ordinary state, so a failed profile creation does not undo the provider.
      await call({
        version: 1, op: "create_profile",
        params: { profile: {
          id, name: name || id,
          tools: Object.fromEntries(toolList.map((t) => [t, { provider: id, slots: {} }])),
        }},
      });
    }
    draw();
  });
}

// -- Profiles --------------------------------------------------------------------------

async function profilesTab(pane) {
  const [{ profiles }, toolList] = await Promise.all([
    call({ version: 1, op: "list_profiles", params: {} }),
    tools(),
  ]);
  pane.innerHTML =
    profiles
      .map(
        (p) => `
      <div class="card" data-id="${esc(p.id)}">
        <div class="row">
          ${tile(p.name)}
          <span class="label">${esc(p.name)}</span>
          <span class="qualifier">${esc(`${p.tools} of ${toolList.length} tools`)}</span>
        </div>
        <div class="actions"><button class="act" data-do="rename">Rename…</button>
          <button class="act" data-do="duplicate">Duplicate…</button>
          <button class="act danger" data-do="delete">Delete profile…</button></div>
        <div class="result note" role="status"></div>
      </div>`
      )
      .join("") +
    `<p class="note">New profiles come from the panel — type an unknown name</p>`;

  pane.querySelectorAll(".card").forEach((card) => {
    const id = card.dataset.id;
    const result = card.querySelector(".result");
    card.querySelectorAll("button").forEach((button) =>
      button.addEventListener("click", () => {
        const doing = button.dataset.do;
        const refused = (r) => {
          result.textContent = r.ok ? "" : `${r.failure.kind} — nothing was changed`;
        };
        if (doing === "rename") {
          askText("Rename…", async (name) => {
            refused(await call({ version: 1, op: "rename_profile", params: { id, name } }));
            if (result.textContent === "") draw();
          });
        } else if (doing === "duplicate") {
          askText("Duplicate…", async (asId) => {
            refused(await call({ version: 1, op: "duplicate_profile", params: { id, as_id: asId } }));
            if (result.textContent === "") draw();
          });
        } else if (doing === "delete") {
          confirmSheet(
            "Delete profile…",
            `Your tools keep what it last applied across ${plural("{count} tool", "{count} tools", p.tools ?? toolList.length)}`,
            async () => {
              refused(await call({ version: 1, op: "delete_profile", params: { id } }));
              if (result.textContent === "") draw();
            }
          );
        }
      })
    );
  });
}

// -- General ---------------------------------------------------------------------------

function general(pane) {
  // Read-only this ticket, recorded in the map: capture-and-rebind is its own ticket.
  pane.innerHTML = `
    <div class="card">
      <div class="row"><span class="label">Shortcuts</span></div>
      <div class="row"><span class="label">Open panel</span><span class="value">⌘⇧P</span></div>
      <div class="row"><span class="label">Cycle profiles</span><span class="value">⌥⌘P</span></div>
    </div>`;
}

// -- The sheet: confirm and ask-text, in page, a real dialog ---------------------------

function sheet(html) {
  return new Promise((resolve) => {
    const host = document.createElement("div");
    host.id = "sheet";
    host.setAttribute("role", "dialog");
    host.setAttribute("aria-modal", "true");
    host.innerHTML = `<div class="box">${html}<div class="actions">
      <button class="act" data-r="0">Cancel</button>
      <button class="act" data-r="1">OK</button>
    </div></div>`;
    document.body.appendChild(host);
    const focusWas = document.activeElement;
    const input = host.querySelector("input");
    (input ?? host.querySelector("[data-r='1']")).focus();
    const close = (ok) => {
      host.remove();
      focusWas?.focus?.();
      resolve(ok ? (input?.value.trim() ?? true) : null);
    };
    host.querySelectorAll("button").forEach((b) =>
      b.addEventListener("click", () => close(b.dataset.r === "1"))
    );
    host.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close(false);
      }
      if (e.key === "Enter" && input) close(true);
      // The trap: tab stays inside the dialog while it exists.
      if (e.key === "Tab") {
        const f = [...host.querySelectorAll("button, input")];
        const first = f[0], last = f[f.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
      }
    });
  });
}

function confirmSheet(title, detail, onOk) {
  sheet(`<strong>${esc(title)}</strong><p class="note">${esc(detail)}</p>`).then((ok) => {
    if (ok) onOk();
  });
}

function askText(title, onOk) {
  sheet(`<strong>${esc(title)}</strong>
    <p class="note">The name is shown; the id never moves.</p>
    <input class="sheet-input" />`).then((value) => {
    if (value && value !== true) onOk(value);
  });
}
