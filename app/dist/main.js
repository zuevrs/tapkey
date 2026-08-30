// One surface per window; the label decides. The panel is a list read in half a second:
// it composes from the row primitive, filters, and switches. Nothing caches.

import { call, esc, plural, tile, tools } from "./ui.js";

const { getCurrentWindow } = window.__TAURI__.window;
const { invoke } = window.__TAURI__.core;

const surface = document.getElementById("surface");
const label = getCurrentWindow().label;

// The platform layer is a token swap, and the OS is the only honest source of which one.
const IS_WIN = /Windows NT/.test(navigator.userAgent);
document.documentElement.dataset.platform = IS_WIN ? "win" : "mac";
document.body.classList.add(label);

if (label === "panel") {
  panel();
} else if (label === "hud") {
  hud();
} else if (label === "settings") {
  import("./settings.js").then((m) => m.settings());
} else if (label === "effective" || label === "history") {
  import("./sheets.js").then((m) =>
    label === "effective" ? m.effectiveState() : m.history()
  );
} else if (label === "onboarding") {
  import("./onboarding.js").then((m) =>
    m.onboarding(() => window.__TAURI__.core.invoke("onboarding_done"))
  );
}

// -- The panel -------------------------------------------------------------------------
//
// The prototype's structure, rendered from the core's facts: a head naming what the tools
// are on, a search opened by its trigger or by typing, sections, rows with ⌘N badges in
// most-recently-used order, a tools section, an attention row when something drifted, and a
// footer whose two openings are global shortcuts. What the prototype shows from demo data,
// this renders from effective_state and list_profiles — and what has no data yet (balances,
// per-tool pinning) is omitted rather than faked, per the design rules.

async function panel() {
  const [profiles, state, toolList] = await Promise.all([
    call({ version: 1, op: "list_profiles", params: {} }),
    call({ version: 1, op: "effective_state", params: {} }),
    tools(),
  ]);
  surface.className = "panel glass";
  current = head(profiles.profiles, state);
  panelState = { rows: profiles.profiles, state, toolList, searchOn: false, query: "", active: -1 };
  drawPanel();

  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      if (toolsOpen) {
        // One layer per press: the tools section peels before the search, the search before
        // the panel — a stray Esc never throws away more than one thing.
        toolsOpen = false;
        drawPanel();
      } else if (panelState.searchOn) {
        // Esc in the search returns to the head; Esc in the head closes the panel — the
        // prototype's two-step, so a stray Esc never throws the panel away.
        panelState.searchOn = false;
        panelState.query = "";
        drawPanel();
      } else {
        getCurrentWindow().hide();
      }
    }
  });
  getCurrentWindow().onFocusChanged(({ payload: focused }) => {
    if (focused && !panelState.searchOn) document.getElementById("srch-btn")?.focus();
  });
}

let panelState = { rows: [], state: { tools: [] }, toolList: [], searchOn: false, query: "", active: -1 };
let current = "Mixed";
let toolsOpen = false;

/// The head: the profile the tools are on, named the way `cycle` finds it — the first owned
/// slot's effective value, matched against the profiles' assignments. No single profile owns
/// every tool's state — the catalogue's word for that is Mixed.
function head(rows, state) {
  const value = state.tools
    ?.find((t) => t.slots?.some((s) => s.owned && s.resolved?.effective))
    ?.slots.find((s) => s.owned && s.resolved?.effective).resolved.effective;
  if (rows.length === 1) return rows[0].name;
  const hit = rows.find((r) => r.name === value || (value && value.includes(r.name)));
  return hit ? hit.name : value ? "Mixed" : "System default";
}

/// Most-recently-used order: the panel answers "switch again to what I just used" first, and
/// ⌘N numbers follow the order the person actually works in. Kept per machine, never synced.
function mruOrder(rows) {
  let order = [];
  try {
    order = JSON.parse(localStorage.getItem("tapkey-mru") || "[]");
  } catch {}
  const byId = new Map(rows.map((r) => [r.id, r]));
  const used = order.filter((id) => byId.has(id)).map((id) => byId.get(id));
  const rest = rows.filter((r) => !used.includes(r));
  return [...used, ...rest];
}
function bumpMru(id) {
  let order = [];
  try {
    order = JSON.parse(localStorage.getItem("tapkey-mru") || "[]");
  } catch {}
  order = [id, ...order.filter((x) => x !== id)].slice(0, 8);
  localStorage.setItem("tapkey-mru", JSON.stringify(order));
}

function drawPanel() {
  const { rows, state, toolList, searchOn, query } = panelState;
  const ordered = mruOrder(rows);
  const visible = searchOn
    ? ordered.filter((r) => !query || r.name.toLowerCase().includes(query.toLowerCase()))
    : ordered;
  const providerOf = (tool) => {
    // The tool's current endpoint, matched against the providers' base URLs — the core's
    // chain knows the endpoint; the panel knows the name.
    const url = tool.endpoint.effective;
    return url ? url.split("/").slice(0, 3).join("/") : null;
  };

  const headHtml = searchOn
    ? `<div class="p-searchrow">
        <input id="search" type="text" role="combobox" aria-expanded="true" aria-controls="list"
               aria-activedescendant="${panelState.active >= 0 ? `opt-${panelState.active}` : ""}"
               placeholder="Switch profile…" aria-label="Switch profile"
               value="${esc(query)}" autocomplete="off" spellcheck="false" /><kbd>esc</kbd>
      </div>`
    : `<header class="p-head">
        ${tile(current)}
        <span class="nm">${esc(current)}</span>
        <button type="button" class="srch" id="srch-btn" title="Filter profiles" aria-label="Filter profiles">⌕</button>
      </header>`;

  const createRow = (searchOn && query && !rows.some((r) => r.name.toLowerCase() === query.toLowerCase()))
    ? `<div class="p-row create" role="option" tabindex="-1" data-create="${esc(query)}"
         title="Creating opens Settings → Profiles">${tile("+")}<span class="nm">Create profile “${esc(query)}”…</span></div>`
    : "";

  const profHtml = visible
    .map((r, i) => `
      <div class="p-row${i === panelState.active ? " sel" : ""}" role="option" id="opt-${i}"
           tabindex="${i === Math.max(panelState.active, 0) ? "0" : "-1"}"
           aria-selected="${i === panelState.active}" data-id="${esc(r.id)}">
        ${tile(r.name)}
        <span class="nm">${esc(r.name)}</span>
        <span class="qual">${esc(`${r.tools} of ${toolList.length} tools`)}</span>
        <span class="kn">⌘${i + 1}</span>
      </div>`)
    .join("") + createRow;

  const drifted = state.tools.some((t) => (t.slots ?? []).some((s) => s.drifted));
  const attnHtml = drifted && !searchOn
    ? `<div class="p-attn" role="status" aria-live="polite">
        <span>${esc(state.tools.find((t) => (t.slots ?? []).some((s) => s.drifted))?.tool ?? "")} — changed outside tapkey
        <span class="acts"><button type="button" class="act pri" id="reapply">Re-apply</button></span></span>
      </div>`
    : "";

  const summary = state.tools.every((t) => t.endpoint.effective === state.tools[0]?.endpoint.effective)
    ? `all on ${esc(current)}`
    : state.tools.map((t) => providerOf(t)).join(" ");
  const toolsHtml = !searchOn
    ? `<div class="p-sechead${toolsOpen ? " open" : ""}" role="button" aria-expanded="${toolsOpen}">
        ${toolList.length} tools <span class="meta">${summary} <span class="chev">›</span></span>
      </div>
      ${toolsOpen ? state.tools.map((t) => `
        <div class="p-row tool" style="margin-left:14px">
          ${tile(cap(t.tool))}<span class="nm">${esc(cap(t.tool))}</span>
          <span class="provlogo">${esc(t.endpoint.effective ?? "—")} <span class="chev">›</span></span>
        </div>
        <div class="p-row sub"><span class="check"></span>
          <span class="nm link" data-effective="1">Effective state…</span></div>`).join("") : ""}`
    : "";

  surface.innerHTML = `
    ${headHtml}
    <div class="p-sec">${searchOn ? "Results" : "Switch to"}</div>
    <div class="p-rows" role="listbox" aria-label="Profiles">${profHtml}</div>
    ${toolsHtml}
    ${attnHtml}
    <footer id="footer">
      <span class="tip">Type to filter</span>
      <button id="open-history">History <kbd>⌘Y</kbd></button>
      <button id="open-settings">Settings <kbd>⌘,</kbd></button>
    </footer>`;

  const search = document.getElementById("search");
  if (searchOn) {
    search.focus();
    search.setSelectionRange(query.length, query.length);
    search.addEventListener("input", () => {
      panelState.query = search.value;
      panelState.active = search.value.trim() ? 0 : -1;
      drawPanel();
    });
    search.addEventListener("keydown", panelKeys);
  } else {
    document.getElementById("srch-btn").addEventListener("click", openSearch);
  }

  surface.querySelectorAll(".p-row[data-id]").forEach((row) =>
    row.addEventListener("click", () => switchTo(row.dataset.id))
  );
  const sechead = surface.querySelector(".p-sechead");
  if (sechead) sechead.addEventListener("click", () => { toolsOpen = !toolsOpen; drawPanel(); });
  surface.querySelectorAll("[data-effective]").forEach((el) =>
    el.addEventListener("click", () => openSheet("effective"))
  );
  document.getElementById("reapply")?.addEventListener("click", (e) => {
    e.stopPropagation();
    switchTo(visible[panelState.active]?.id ?? visible[0]?.id);
  });
  document.getElementById("open-history").addEventListener("click", () => openSheet("history"));
  document.getElementById("open-settings").addEventListener("click", () => openSheet("settings"));

  // Any typing in the head opens the search — the palette idiom: the panel's whole
  // interaction is the field, and the person should never have to find the trigger first.
  if (!searchOn) {
    surface.addEventListener("keydown", (e) => {
      if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
        openSearch(e.key);
      }
    });
  }
}

function openSearch(seed) {
  panelState.searchOn = true;
  panelState.query = typeof seed === "string" ? seed : "";
  panelState.active = panelState.query ? 0 : -1;
  drawPanel();
}

/// The keyboard: arrows walk the rows (wrapping, Home/End), Enter switches the walked-to
/// row, digits 1-9 with ⌘ switch by badge.
function panelKeys(e) {
  const rows = [...surface.querySelectorAll(".p-row")];
  const count = rows.length;
  const move = (to) => {
    if (!count) return;
    panelState.active = (to + count) % count;
    drawPanel();
  };
  if (e.key === "ArrowDown") { e.preventDefault(); move(panelState.active < 0 ? 0 : panelState.active + 1); }
  else if (e.key === "ArrowUp") { e.preventDefault(); move(panelState.active < 0 ? count - 1 : panelState.active - 1); }
  else if (e.key === "Home") { e.preventDefault(); move(0); }
  else if (e.key === "End") { e.preventDefault(); move(count - 1); }
  else if (e.key === "Enter") {
    const row = rows[panelState.active >= 0 ? panelState.active : 0];
    if (row) row.click();
  } else if (/^[1-9]$/.test(e.key) && (e.metaKey || e.ctrlKey)) {
    const row = rows[Number(e.key) - 1];
    if (row) row.click();
  }
}

function openSheet(sheet) {
  if (sheet === "settings") {
    window.__TAURI__.window.Window.getByLabel("settings")?.show();
  } else {
    window.__TAURI__.core.invoke("show_sheet", { sheet });
  }
}

async function switchTo(id) {
  // The response names the backup the switch took (core ticket 33); the HUD's Undo restores
  // exactly that. The HUD drives itself from the query parameters.
  const response = await call({ version: 1, op: "switch", params: { profile_id: id } });
  bumpMru(id);
  await invoke("show_hud", {
    responseJson: JSON.stringify(response),
    backupId: response.backup ?? "",
  });
  getCurrentWindow().hide();
}

// -- The HUD window --------------------------------------------------------------------

function hud() {
  const params = new URLSearchParams(location.search);
  const response = JSON.parse(params.get("response") || "{}");
  const backup = params.get("backup");
  const applied = response.outcome === "applied";
  // The design rules settle the default: an unmeasured reload behaviour is "on next launch",
  // in neutral colour — a permanent amber glyph teaches users to ignore amber. And a notice
  // reporting a failure carries no timer: only success fades on its own.
  const result = applied
    ? "on next launch"
    : response.outcome === "rolled back"
      ? "Switch rolled back"
      : "Switch failed";
  const timed = applied;
  surface.className = "hud glass";
  surface.innerHTML = `
    <span class="result" role="status">${esc(result)}</span>
    ${applied && backup ? `<button id="undo">Undo</button>` : ""}`;
  if (applied && backup) {
    document.getElementById("undo").addEventListener("click", async () => {
      // Undo is a core operation like any other: one envelope through the bridge, never a
      // second marshalling of a request in the app.
      const r = await call({
        version: 1, op: "restore",
        params: { target: { target: "backup", id: backup } },
      });
      document.querySelector(".result").textContent = r.outcome === "applied" ? "Restored" : "Switch failed";
      document.getElementById("undo").remove();
    });
  }
  if (timed) setTimeout(() => getCurrentWindow().hide(), 4000);
}
