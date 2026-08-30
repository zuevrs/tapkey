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
} else if (label === "onboarding") {
  import("./onboarding.js").then((m) =>
    m.onboarding(() => window.__TAURI__.core.invoke("onboarding_done"))
  );
}

// -- The panel -------------------------------------------------------------------------

async function panel() {
  const [profiles, state, toolList] = await Promise.all([
    call({ version: 1, op: "list_profiles", params: {} }),
    call({ version: 1, op: "effective_state", params: {} }),
    tools(),
  ]);
  surface.className = "panel";
  render(profiles.profiles, toolList);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") getCurrentWindow().hide();
  });
}

function render(rows, toolList) {
  surface.innerHTML = `
    <input id="search" type="text" placeholder="Switch profile…" aria-label="Switch profile" />
    <div id="list" role="listbox" aria-label="Profiles"></div>
    <div id="footer" aria-hidden="true">Type to filter · Enter to switch · Esc to close</div>`;
  const search = document.getElementById("search");
  const list = document.getElementById("list");

  const draw = () => {
    const query = search.value.trim().toLowerCase();
    const visible = rows.filter((r) => !query || r.name.toLowerCase().includes(query));
    list.innerHTML = visible
      .map(
        (r, i) => `
      <div class="row${query && i === 0 ? " active" : ""}" role="option" tabindex="-1"
           aria-selected="${query && i === 0}" data-id="${esc(r.id)}">
        ${tile(r.name)}
        <span class="label">${esc(r.name)}</span>
        <span class="qualifier">${esc(`${r.tools} of ${toolList.length} tools`)}</span>
        <span class="value">↩</span>
      </div>`
      )
      .join("");
    if (query && !rows.some((r) => r.name.toLowerCase() === query)) {
      list.insertAdjacentHTML(
        "beforeend",
        `<div class="row" role="option" tabindex="-1" data-create="${esc(search.value.trim())}"
             title="Creating opens Settings → Profiles">
           ${tile("+")}
           <span class="label">Create profile “${esc(search.value.trim())}”…</span>
         </div>`
      );
    }
    list.querySelectorAll(".row").forEach((row) =>
      row.addEventListener("click", () =>
        row.dataset.create ? undefined : switchTo(row.dataset.id)
      )
    );
  };

  search.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      const first = list.querySelector(".row");
      if (first) first.click();
    }
  });
  search.addEventListener("input", draw);
  draw();
  search.focus();
}

async function switchTo(id) {
  // The response names the backup the switch took (core ticket 33); the HUD's Undo restores
  // exactly that. The HUD drives itself from the query parameters.
  const response = await call({ version: 1, op: "switch", params: { profile_id: id } });
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
      ? "Switch rolled back — restored"
      : "Switch failed";
  const timed = applied;
  surface.className = "hud";
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
