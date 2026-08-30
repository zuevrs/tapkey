// Settings: Providers, Profiles, General. Every action is a core operation through the bridge;
// the surface composes and never decides. Destructive actions confirm in a page sheet, because
// WKWebView does not implement window.confirm and the styling is ours anyway.

import { call, esc, tile, tools, cap, plural, slotName, slotHint, PRESETS } from "./ui.js";

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

let selected = null; // the side list's selection: a provider id, or null for the presets view

async function draw() {
  surface.innerHTML = `
    <nav class="tabs">
      <button data-tab="providers">Providers</button>
      <button data-tab="profiles">Profiles</button>
      <button data-tab="general">General</button>
    </nav>
    <div class="win-body" id="win-body">
      <aside class="side" id="side-list" ${tab === "general" ? 'style="display:none"' : ""}></aside>
      <section class="pane" id="pane-content"></section>
    </div>`;
  surface.querySelectorAll(".tabs button").forEach((b) =>
    b.addEventListener("click", () => {
      tab = b.dataset.tab;
      selected = null;
      draw();
    })
  );
  surface.querySelector(`[data-tab="${tab}"]`).classList.add("active");
  const side = document.getElementById("side-list");
  const pane = document.getElementById("pane-content");
  if (tab === "providers") await providers(side, pane);
  else if (tab === "profiles") await profilesTab(side, pane);
  else general(pane);
}

// -- Providers -------------------------------------------------------------------------
//
// The prototype's architecture: a side list to find and choose, a presets view for adding
// (official providers first, Custom endpoint for marketplaces), and a detail pane whose
// groups — Provider, Endpoint, Connection — carry the keychain truth and the format facts.
// Balance and Models have no data and no core operation yet: omitted rather than faked, per
// the design rules and A12's record.

async function providers(side, pane) {
  const [{ providers: list }, toolList] = await Promise.all([
    call({ version: 1, op: "list_providers", params: {} }),
    tools(),
  ]);
  const all = list;
  const { profiles } = await call({ version: 1, op: "list_profiles", params: {} });
  const using = {};
  for (const prof of profiles)
    for (const assignment of Object.values(prof.assignments ?? {}))
      if (assignment.provider) using[assignment.provider] = (using[assignment.provider] ?? 0) + 1;
  drawSide(side, all.map((p) => ({ id: p.id, name: p.name, on: selected === p.id })),
    "Find a provider…", "Add provider…", () => { selected = null; draw(); });
  if (!selected || !all.some((p) => p.id === selected)) {
    renderPresets(pane);
  } else {
    renderProviderPane(pane, all.find((p) => p.id === selected), using, toolList);
  }
}

/// The side list both tabs share: a find field, the items, and the add foot. Finding filters
/// the list; nothing here reads or holds a secret.
function drawSide(side, items, placeholder, addLabel, onAdd) {
  side.innerHTML = `
    <input class="side-find" placeholder="${esc(placeholder)}" aria-label="${esc(placeholder)}" />
    <div role="listbox" aria-label="Items">${items
      .map((it) => `<div class="s-item${it.on ? " on" : ""}" role="option" tabindex="-1"
           aria-selected="${it.on}" data-id="${esc(it.id)}"><span>${esc(it.name)}</span></div>`)
      .join("")}</div>
    <div class="plus" role="button" tabindex="0">＋ ${esc(addLabel)}</div>`;
  const find = side.querySelector(".side-find");
  find.addEventListener("input", () => {
    const q = find.value.trim().toLowerCase();
    side.querySelectorAll(".s-item").forEach((el) => {
      el.style.display = !q || el.textContent.toLowerCase().includes(q) ? "" : "none";
    });
  });
  side.querySelectorAll(".s-item").forEach((el) =>
    el.addEventListener("click", () => { selected = el.dataset.id; draw(); })
  );
  side.querySelector(".plus").addEventListener("click", onAdd);
}

/// The presets view: official providers as rows, the add-what group, the note.
function renderPresets(pane) {
  pane.innerHTML = `
    <div class="pane-head">Add provider</div>
    <div class="g-label">Official providers</div>
    <div class="group">
      ${PRESETS.map((p, i) => `
        <div class="g-row preset" data-pi="${i}" role="button" tabindex="0" style="cursor:pointer">
          ${tile(p.name)}
          <span class="gl" style="min-width:0">${esc(p.name)}</span>
          <span class="gv">${p.opt ? `<span class="hint">${esc(p.opt)}</span>` : ""}<span class="hint">›</span></span>
        </div>`).join("")}
    </div>
    <div class="g-label">On add</div>
    <div class="group">
      <div class="g-row">
        <span class="gl" style="min-width:0">Also create a profile</span>
        <span class="gv"><span class="hint">Switchable right away</span>
          <label class="swl"><input type="checkbox" id="preset-auto" checked /><span class="sw"></span></label></span>
      </div>
    </div>
    <div class="g-note">Marketplaces and resellers use Custom endpoint</div>`;
  pane.querySelectorAll(".preset").forEach((el) =>
    el.addEventListener("click", () => {
      const preset = PRESETS[Number(el.dataset.pi)];
      if (preset.name.startsWith("Custom")) renderNewProvider(pane);
      else addPreset(pane, preset);
    })
  );
}

async function addPreset(pane, preset) {
  const id = preset.name.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  const created = await call({
    version: 1, op: "create_provider",
    params: { id, name: preset.name, base_url: preset.base_url },
  });
  if (!created.ok) {
    pane.insertAdjacentHTML("beforeend",
      `<div class="g-note">${esc(created.failure?.kind ?? "refused")} — nothing was changed</div>`);
    return;
  }
  if (document.getElementById("preset-auto")?.checked) {
    const toolList = await tools();
    await call({
      version: 1, op: "create_profile",
      params: { profile: {
        id, name: preset.name,
        tools: Object.fromEntries(toolList.map((t) => [t, { provider: id, slots: {} }])),
      }},
    });
  }
  selected = id;
  draw();
}

/// The custom form, Q9-Q11's shape: name, base URL, key, and Test; format is a result, not
/// an input.
function renderNewProvider(pane) {
  pane.innerHTML = `
    <div class="pane-head">${tile("+")}New provider</div>
    <div class="g-label">Endpoint</div>
    <div class="g-desc">Where requests go, and the key that signs them</div>
    <div class="group">
      <div class="g-row"><span class="gl">Name</span>
        <span class="gv grow"><input class="tfield grow" id="np-name" placeholder="e.g. Work OpenRouter" /></span></div>
      <div class="g-row"><span class="gl">Base URL</span>
        <span class="gv grow"><input class="tfield mono grow" id="np-url" placeholder="https://api.example.com/v1" /></span></div>
      <div class="g-row"><span class="gl">API key</span>
        <span class="gv grow"><input class="tfield mono grow" id="np-key" type="password" placeholder="Paste a key" /></span></div>
      <div class="g-row"><span class="gl">API format</span>
        <span class="gv"><span class="hint">Unknown until you test</span></span></div>
      <div class="g-status">Test fills this in</div>
      <div class="g-row"><span class="gl">Connection</span>
        <span class="gv"><span class="hint">Not tested yet</span>
          <button class="act" id="np-add">Add provider</button></span></div>
    </div>
    <div class="g-note">You can add it untested — Test later fills the format</div>
    <div class="g-note" id="np-result" role="status"></div>`;
  document.getElementById("np-add").addEventListener("click", async () => {
    const name = document.getElementById("np-name").value.trim();
    const base_url = document.getElementById("np-url").value.trim();
    const key = document.getElementById("np-key").value.trim();
    const id = name.toLowerCase().replace(/[^a-z0-9]+/g, "-") || `provider-${Date.now()}`;
    const created = await call({
      version: 1, op: "create_provider",
      params: { id, name: name || id, base_url },
    });
    if (!created.ok) {
      document.getElementById("np-result").textContent = `${created.failure.kind} — nothing was changed`;
      return;
    }
    if (key) {
      const stored = await call({ version: 1, op: "set_credential", params: { provider_id: id, secret: key } });
      if (!stored.ok) {
        document.getElementById("np-result").textContent = `${stored.failure.kind} — nothing was changed`;
        return;
      }
    }
    selected = id;
    draw();
  });
}

/// The detail pane: groups Provider / Endpoint / Connection, the keychain truth, the format
/// facts, the real Test, and the delete with its consequence counted from the store.
function renderProviderPane(pane, p, using, toolList) {
  const format = !p.formats
    ? null
    : p.formats.length === 0
      ? "Serves none of your tools"
      : p.formats.length === toolList.length
        ? `Serves all ${p.formats.length} tools`
        : `Serves ${p.formats.join(", ")}`;
  const reachesOpenCode = p.formats?.some((f) => f === "openai_chat") ?? true;
  const fmtNames = { anthropic_messages: "Anthropic Messages", openai_responses: "OpenAI Responses", openai_chat: "OpenAI Chat" };
  const result = pane.querySelector("#pane-result");

  const draw2 = (testNote) => {
    const usedBy = using[p.id] ?? 0;
    pane.innerHTML = `
      <div class="pane-head">${tile(p.name)}${esc(p.name)}</div>
      <div class="g-label">Provider</div>
      <div class="group">
        <div class="g-row"><span class="gl">Name</span>
          <span class="gv grow"><span class="tfield grow">${esc(p.name)}</span></span></div>
      </div>
      <div class="g-label">Endpoint</div>
      <div class="g-desc">Where requests go, and the key that signs them</div>
      <div class="group">
        <div class="g-row"><span class="gl">Base URL</span>
          <span class="gv grow"><span class="tfield mono grow">${esc(p.base_url)}</span></span></div>
        <div class="g-row"><span class="gl">API key</span>
          <span class="gv grow"><span class="tfield mono grow">••••••••••••••••</span></span></div>
        <div class="g-status">${reachesOpenCode
          ? "OpenCode has no key helper — written to a file only you can read"
          : "Keychain · Claude Code and Codex fetch it through a helper command"}</div>
        <div class="g-row"><span class="gl">API format</span>
          <span class="gv">${format ? `<span>${esc(format)}</span>` : '<span class="hint">Unknown until you test</span>'}</span></div>
        <div class="g-status">${format ? p.formats.map((f) => fmtNames[f] ?? f).join(" · ") : "Test fills this in"}</div>
        <div class="g-row"><span class="gl">Connection</span>
          <span class="gv">${p.formats ? `<span class="hint"><span class="ok">✓</span> ${esc(testNote ?? "tested")}</span>`
            : '<span class="hint">Not tested yet</span>'}
            <span class="hint" id="test-out"></span><button class="act" id="test-btn">Test</button></span></div>
        <div class="g-row"><span class="gl">Enabled</span>
          <span class="gv"><label class="swl"><input type="checkbox" id="prov-enabled" ${p.enabled ? "checked" : ""}/><span class="sw"></span></label></span></div>
      </div>
      <div class="group"><button class="act danger" id="remove-btn">Delete provider…</button></div>
      <div class="g-note">${usedBy ? `${usedBy} ${usedBy === 1 ? "profile uses" : "profiles use"} this provider and will fall back to System default` : ""}</div>
      <div class="g-note" id="pane-result" role="status"></div>`;

    document.getElementById("test-btn").addEventListener("click", async () => {
      document.getElementById("test-out").textContent = "Testing…";
      const started = performance.now();
      const r = await call({ version: 1, op: "test", params: { provider_id: p.id } });
      const note = !r.knowable
        ? "This endpoint answers everything — its formats can’t be read from here"
        : r.formats.some((f) => f.served)
          ? `${Math.round(performance.now() - started)} ms · answers ${r.formats.filter((f) => f.served).map((f) => f.format).join(", ")}`
          : "Serves none of your tools";
      document.getElementById("test-out").textContent = "";
      draw2(note);
    });
    document.getElementById("prov-enabled").addEventListener("change", async (e) => {
      const r = await call({ version: 1, op: "set_provider_enabled", params: { id: p.id, enabled: e.target.checked } });
      if (!r.ok) {
        document.getElementById("pane-result").textContent = `${r.failure.kind} — nothing was changed`;
        e.target.checked = !e.target.checked;
      }
    });
    document.getElementById("remove-btn").addEventListener("click", () => {
      confirmSheet(
        "Delete provider…",
        `Its entries go from ${toolList.map(cap).join(", ")}; the key tapkey stored is deleted`,
        async () => {
          const r = await call({ version: 1, op: "remove_provider", params: { id: p.id } });
          if (r.ok) { selected = null; draw(); }
          else document.getElementById("pane-result").textContent = `${r.failure.kind} — nothing was changed`;
        }
      );
    });
  };
  draw2();
}

// -- Profiles --------------------------------------------------------------------------
//
// The side list chooses; the pane is the editor — the prototype's renderProfile is a detail
// pane, not a stack of cards, and Rename/Duplicate/Delete live in the pane's own actions.

async function profilesTab(side, pane) {
  const [{ profiles }, toolList] = await Promise.all([
    call({ version: 1, op: "list_profiles", params: {} }),
    tools(),
  ]);
  if (!selected || !profiles.some((p) => p.id === selected)) {
    selected = profiles[0]?.id ?? null;
  }
  drawSide(side, profiles.map((p) => ({ id: p.id, name: p.name, on: selected === p.id })),
    "Find a profile…", "New profile…", () => {
      pane.innerHTML = `
        <div class="pane-head">New profile</div>
        <div class="g-desc">New profiles come from the panel — type an unknown name</div>`;
    });
  const profile = profiles.find((p) => p.id === selected);
  if (!profile) {
    pane.innerHTML = `
      <div class="pane-head">Profiles</div>
      <div class="g-desc">New profiles come from the panel — type an unknown name</div>`;
    return;
  }
  editProfile(profile.id, pane, toolList);
  pane.insertAdjacentHTML("beforeend", `
    <div class="group">
      <div class="actions">
        <button class="act" id="pf-rename">Rename…</button>
        <button class="act" id="pf-duplicate">Duplicate…</button>
        <button class="act danger" id="pf-delete">Delete profile…</button>
      </div>
    </div>
    <div class="g-note" id="pf-result" role="status"></div>`);
  const result = pane.querySelector("#pf-result");
  const refused = (r) => { result.textContent = r.failure ? `${r.failure.kind} — nothing was changed` : ""; };
  document.getElementById("pf-rename").addEventListener("click", () => {
    askText("Rename…", async (name) => {
      refused(await call({ version: 1, op: "rename_profile", params: { id: profile.id, name } }));
      if (!result.textContent) draw();
    });
  });
  document.getElementById("pf-duplicate").addEventListener("click", () => {
    askText("Duplicate…", async (asId) => {
      refused(await call({ version: 1, op: "duplicate_profile", params: { id: profile.id, as_id: asId } }));
      if (!result.textContent) { selected = asId; draw(); }
    });
  });
  document.getElementById("pf-delete").addEventListener("click", () => {
    confirmSheet(
      "Delete profile…",
      `Your tools keep what it last applied across ${plural("{count} tool", "{count} tools", profile.tools ?? toolList.length)}`,
      async () => {
        refused(await call({ version: 1, op: "delete_profile", params: { id: profile.id } }));
        if (!result.textContent) { selected = null; draw(); }
      }
    );
  });
}

// -- General ---------------------------------------------------------------------------

function general(pane) {
  pane.innerHTML = `
    <div class="pane-head">General</div>
    <div class="g-label">Menu bar</div>
    <div class="group">
      <div class="g-row">
        <span class="gl">Show profile name</span>
        <span class="gv"><span class="hint">The glyph alone stays legible anywhere</span></span>
      </div>
      <div class="g-status">Waits on the glyph-menu ticket — the tooltip names the profile once
        the tray keeps live state</div>
    </div>
    <div class="g-label">Shortcuts</div>
    <div class="group">
      <div class="g-row"><span class="gl">Open panel</span><span class="gv"><span class="tfield mono">⌘⇧P</span></span></div>
      <div class="g-row"><span class="gl">Cycle profiles</span><span class="gv"><span class="tfield mono">⌥⌘P</span></span></div>
    </div>
    <div class="g-label">Safety</div>
    <div class="group">
      <div class="g-row">
        <span class="gl">Watch configs</span>
        <span class="gv"><span class="hint">Notice edits made outside tapkey</span></span>
      </div>
      <div class="g-status">Waits on the file-watching ticket — the panel reads state on every
        open, which is fresh and costs ~150µs</div>
      <div class="g-row">
        <span class="gl">Backups</span>
        <span class="gv"><span class="hint">Keep the last 10 switches</span></span>
      </div>
    </div>
    <div class="g-label">Notifications</div>
    <div class="group">
      <div class="g-row">
        <span class="gl">Switch failed</span>
        <span class="gv"><span class="hint">Rare, and needs a decision</span></span>
      </div>
      <div class="g-row">
        <span class="gl">Low balance</span>
      </div>
      <div class="g-status">Both wait on the notification machinery — the HUD carries failures
        until then</div>
    </div>
    <div class="g-label">Startup</div>
    <div class="group">
      <div class="g-row">
        <span class="gl">Open at login</span>
        <span class="gv"><label class="swl"><input type="checkbox" id="g-autostart" /><span class="sw"></span></label></span>
      </div>
      <div class="g-row">
        <span class="gl">Run setup again…</span>
        <span class="gv"><button class="act" id="g-setup">Run setup again…</button></span>
      </div>
    </div>`;
  window.__TAURI__.core.invoke("get_autostart").then((on) => {
    document.getElementById("g-autostart").checked = on;
  });
  document.getElementById("g-autostart").addEventListener("change", async (e) => {
    try {
      const now = await window.__TAURI__.core.invoke("set_autostart", { enabled: e.target.checked });
      e.target.checked = now;
    } catch {
      e.target.checked = !e.target.checked;
    }
  });
  document.getElementById("g-setup").addEventListener("click", () => {
    window.__TAURI__.core.invoke("run_setup_again");
  });
}

// -- The profile editor -----------------------------------------------------------------
//
// The slot inventory is the adapter's fact — effective_state names each tool's owned slots —
// and the assignments are the core's (they ride the profile row). The editor owns nothing but
// the editing: a working copy in memory, Save sending the whole shape through update_profile,
// an empty slot meaning the ADR's own null assignment — *no assignment*, an instruction, not
// an absence of one.

async function editProfile(id, paneArg, toolListArg) {
  const pane = paneArg ?? document.getElementById("tab");
  const [{ profiles, providers }, state] = await Promise.all([
    call({ version: 1, op: "list_profiles", params: {} }),
    call({ version: 1, op: "effective_state", params: {} }),
  ]);
  const profile = profiles.find((p) => p.id === id);
  if (!profile) return;

  const working = JSON.parse(JSON.stringify(profile.assignments ?? {}));
  let custom = false;
  const enabled = providers.filter((p) => p.enabled);

  const draw = () => {
    const first = state.tools[0];
    const anyProvider = Object.values(working)[0]?.provider ?? "";
    const mainOf = (tool) => working[tool]?.slots?.main ?? "";
    pane.innerHTML = `
      <header class="ob-title">${esc(profile.name)}</header>
      <p class="g-desc">The tools this profile changes; the rest keep what they have</p>
      <div class="g-label">All tools</div>
      <div class="group">
        <div class="row tall">
          <span class="label">Provider</span>
          <span class="trail"><select id="uni-provider">
            <option value="">Not in this profile</option>
            ${enabled.map((p) =>
              `<option value="${esc(p.id)}"${anyProvider === p.id ? " selected" : ""}>${esc(p.name)}</option>`).join("")}
          </select></span>
        </div>
        <div class="row tall">
          <span class="stack"><span class="label">Main model</span></span>
          <span class="trail"><input class="sheet-input" id="uni-main" value="${esc(mainOf(first?.tool))}" placeholder="—" /></span>
        </div>
      </div>
      <div class="actions"><button class="act" id="customize">${custom ? "Hide per-tool detail" : "Customize per tool…"}</button></div>
      ${custom ? state.tools.map((tool) => {
        const a = working[tool.tool] ?? { provider: null, slots: {} };
        return `<div class="g-label">${esc(cap(tool.tool))}</div>
        <div class="group">
          <div class="row tall">
            <span class="label">Provider</span>
            <span class="trail"><select data-tool="${esc(tool.tool)}" data-kind="provider">
              <option value="">Not in this profile</option>
              ${enabled.map((p) =>
                `<option value="${esc(p.id)}"${a.provider === p.id ? " selected" : ""}>${esc(p.name)}</option>`).join("")}
            </select></span>
          </div>
          ${tool.slots.filter((s) => s.owned).map((slot) => `
            <div class="row tall">
              <span class="stack"><span class="label">${esc(slotName(slot.slot))}</span>
                ${slotHint(slot.slot) ? `<span class="qualifier">${esc(slotHint(slot.slot))}</span>` : ""}</span>
              <span class="trail"><input class="sheet-input" data-tool="${esc(tool.tool)}" data-slot="${esc(slot.slot)}"
                   value="${esc(a.slots?.[slot.slot] ?? "")}" placeholder="—" /></span>
            </div>`).join("")}
        </div>`;
      }).join("") : ""}
      <div class="ob-foot">
        <button class="act" id="edit-cancel">Cancel</button>
        <button class="act primary" id="edit-save">Save</button>
      </div>
      <div class="result note" role="status"></div>`;

    // The uniform row is the default the catalogue draws: one provider, one main model, every
    // tool. Setting them writes through to each tool's assignment at once.
    pane.querySelector("#uni-provider").addEventListener("change", (e) => {
      const provider = e.target.value || null;
      for (const tool of state.tools) {
        if (!provider) {
          delete working[tool.tool];
        } else if (!working[tool.tool]) {
          working[tool.tool] = { provider, slots: {} };
        } else {
          working[tool.tool].provider = provider;
        }
      }
    });
    pane.querySelector("#uni-main").addEventListener("change", (e) => {
      const model = e.target.value.trim() || null;
      for (const tool of state.tools) {
        if (working[tool.tool]) working[tool.tool].slots.main = model;
      }
    });
    pane.querySelector("#customize").addEventListener("click", () => {
      custom = !custom;
      draw();
    });
    pane.querySelectorAll("select[data-kind=provider]").forEach((sel) =>
      sel.addEventListener("change", () => {
        const tool = sel.dataset.tool;
        if (!sel.value) {
          delete working[tool];
        } else if (!working[tool]) {
          working[tool] = { provider: sel.value, slots: {} };
        } else {
          working[tool].provider = sel.value;
        }
      })
    );
    pane.querySelectorAll("input[data-slot]").forEach((input) =>
      input.addEventListener("change", () => {
        const tool = working[input.dataset.tool];
        if (!tool) return;
        // Empty is the ADR's null assignment: a deliberate *no assignment*, which for Claude
        // Code neutralises a shell export rather than merely declining to write one.
        tool.slots[input.dataset.slot] = input.value.trim() || null;
      })
    );
    document.getElementById("edit-cancel").addEventListener("click", () => draw());
    document.getElementById("edit-save").addEventListener("click", async () => {
      const result = pane.querySelector(".result");
      const r = await call({
        version: 1, op: "update_profile",
        params: { id, tools: working },
      });
      if (r.ok) draw();
      else result.textContent = `${r.failure.kind} — nothing was changed`;
    });
  };
  draw();
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
