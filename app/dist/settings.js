// Settings: Providers, Profiles, General. Every action is a core operation through the bridge;
// the surface composes and never decides. Destructive actions confirm in a page sheet, because
// WKWebView does not implement window.confirm and the styling is ours anyway.

const { invoke } = window.__TAURI__.core;

const surface = document.getElementById("surface");
const call = (request) =>
  invoke("invoke", { request: JSON.stringify(request) }).then(JSON.parse);

const esc = (text) =>
  String(text).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

let tab = "providers";

export function settings() {
  document.getElementById("surface").className = "settings";
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
  draw();
}

async function draw() {
  const pane = document.getElementById("tab");
  if (tab === "providers") await providers(pane);
  else if (tab === "profiles") await profilesTab(pane);
  else general(pane);
}

// -- Providers -------------------------------------------------------------------------

async function providers(pane) {
  const { providers } = await call({ version: 1, op: "list_providers", params: {} });
  const tools = ["claude", "codex", "opencode"];
  pane.innerHTML =
    providers
      .map((p) => {
        const format = !p.formats
          ? "Unknown until you test"
          : p.formats.length === 0
            ? "Serves none of your tools"
            : p.formats.length === tools.length
              ? `Serves all ${p.formats.length} tools`
              : `Serves ${p.formats.join(", ")}`;
        const keyLine =
          p.formats?.includes("openai_chat") || p.formats?.includes("openai_responses")
            ? "Claude Code and Codex fetch it through a helper command"
            : "Stored in the Keychain, never in a config file";
        return `
      <div class="card" data-id="${esc(p.id)}">
        <div class="row">
          <div class="tile">${esc(p.name.slice(0, 1).toUpperCase())}</div>
          <span class="label">${esc(p.name)}</span>
          <span class="value">${esc(p.enabled ? "" : "off")}</span>
        </div>
        <div class="row">
          <span class="label">Endpoint</span>
          <span class="value">${esc(p.base_url)}</span>
        </div>
        <div class="row">
          <span class="label">API format</span>
          <span class="value">${esc(format)}</span>
        </div>
        <div class="row"><span class="note">${esc(keyLine)}</span></div>
        <div class="actions" style="display:flex;gap:8px;margin-top:8px">
          <button class="act" data-do="test">Test</button>
          <button class="act" data-do="toggle">${p.enabled ? "Turn off" : "Turn on"}</button>
          <button class="act danger" data-do="remove">Remove…</button>
        </div>
        <div class="result note"></div>
      </div>`;
      })
      .join("") +
    `
    <div class="card">
      <div class="field"><span>Name</span><input id="np-name" placeholder="e.g. Work OpenRouter" /></div>
      <div class="field"><span>Base URL</span><input id="np-url" placeholder="https://api.example.com/v1" /></div>
      <div class="field"><span>API key</span><input id="np-key" type="password" placeholder="Paste a key" /></div>
      <label class="field"><input type="checkbox" id="np-auto" checked /> Also create a profile — switchable right away</label>
      <button class="act" id="np-add">Add provider</button>
      <div class="result note"></div>
    </div>`;

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
        } else if (doing === "remove") {
          confirmSheet(
            "Remove provider…",
            "Its entries go from your tools; the key tapkey stored is deleted.",
            async () => {
              const r = await call({ version: 1, op: "remove_provider", params: { id } });
              if (r.ok) draw();
              else result.textContent = r.failure?.detail ?? "Refused";
            }
          );
        }
      })
    );
  });

  pane.querySelector("#np-add").addEventListener("click", async () => {
    const result = pane.querySelectorAll(".card .result")[pane.querySelectorAll(".card").length - 1];
    const name = pane.querySelector("#np-name").value.trim();
    const base = pane.querySelector("#np-url").value.trim();
    const key = pane.querySelector("#np-key").value.trim();
    const id = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || `provider-${Date.now()}`;
    const created = await call({
      version: 1, op: "create_provider",
      params: { id, name: name || id, base_url: base },
    });
    if (!created.ok) {
      result.textContent = created.failure?.detail ?? "Refused";
      return;
    }
    if (key) {
      const stored = await call({
        version: 1, op: "set_credential", params: { provider_id: id, secret: key },
      });
      if (!stored.ok) {
        result.textContent = stored.failure?.detail ?? "The key was refused";
        return;
      }
    }
    if (pane.querySelector("#np-auto").checked) {
      // Two calls joined by the interface, per ticket 31: a provider without a profile is an
      // ordinary state, so a failed profile creation does not undo the provider.
      await call({
        version: 1, op: "create_profile",
        params: { profile: {
          id, name: name || id,
          tools: Object.fromEntries(["claude", "codex", "opencode"].map((t) => [t, { provider: id, slots: {} }])),
        }},
      });
    }
    draw();
  });
}

// -- Profiles --------------------------------------------------------------------------

async function profilesTab(pane) {
  const { profiles } = await call({ version: 1, op: "list_profiles", params: {} });
  pane.innerHTML =
    profiles
      .map(
        (p) => `
      <div class="card" data-id="${esc(p.id)}">
        <div class="row">
          <div class="tile">${esc(p.name.slice(0, 1).toUpperCase())}</div>
          <span class="label">${esc(p.name)}</span>
          <span class="qualifier">${esc(`${p.tools} of 3 tools`)}</span>
        </div>
        <div class="actions" style="display:flex;gap:8px;margin-top:8px">
          <button class="act" data-do="rename">Rename…</button>
          <button class="act" data-do="duplicate">Duplicate…</button>
          <button class="act danger" data-do="delete">Delete…</button>
        </div>
        <div class="result note"></div>
      </div>`
      )
      .join("") +
    `<p class="note">New profiles come from the panel — type an unknown name — or from onboarding.</p>`;

  pane.querySelectorAll(".card").forEach((card) => {
    const id = card.dataset.id;
    const result = card.querySelector(".result");
    card.querySelectorAll("button").forEach((button) =>
      button.addEventListener("click", () => {
        const doing = button.dataset.do;
        if (doing === "rename") {
          askText("Rename…", async (name) => {
            const r = await call({ version: 1, op: "rename_profile", params: { id, name } });
            if (r.ok) draw();
            else result.textContent = r.failure?.detail ?? "Refused";
          });
        } else if (doing === "duplicate") {
          askText("Duplicate…", async (asId) => {
            const r = await call({ version: 1, op: "duplicate_profile", params: { id, as_id: asId } });
            if (r.ok) draw();
            else result.textContent = r.failure?.detail ?? "Refused";
          });
        } else if (doing === "delete") {
          confirmSheet(
            "Delete profile…",
            "Your tools keep what it last applied; they stop having a name for it.",
            async () => {
              const r = await call({ version: 1, op: "delete_profile", params: { id } });
              if (r.ok) draw();
              else result.textContent = r.failure?.detail ?? "Refused";
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

// -- The sheet: confirm and ask-text, in page, because WKWebView implements neither ----------------

function sheet(html) {
  return new Promise((resolve) => {
    const host = document.createElement("div");
    host.id = "sheet";
    host.innerHTML = `<div class="box">${html}<div class="actions">
      <button class="act" data-r="0">Cancel</button>
      <button class="act" data-r="1">OK</button>
    </div></div>`;
    document.body.appendChild(host);
    host.querySelectorAll("button").forEach((b) =>
      b.addEventListener("click", () => {
        const value = host.querySelector("input")?.value.trim();
        host.remove();
        resolve(b.dataset.r === "1" ? (value ?? true) : null);
      })
    );
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
    <input style="width:100%;padding:7px 10px;margin-top:8px;font-size:13px;color:inherit;background:transparent;border:1px solid color-mix(in srgb, currentColor 20%, transparent);border-radius:6px" />`).then(
    (value) => {
      if (value && value !== true) onOk(value);
    }
  );
}
