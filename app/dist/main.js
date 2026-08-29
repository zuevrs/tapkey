// One surface per window; the label decides. The panel is a list read in half a second:
// it composes from the row primitive, filters, and switches. Nothing caches.

const { getCurrentWindow } = window.__TAURI__.window;
const { invoke } = window.__TAURI__.core;

const surface = document.getElementById("surface");
const label = getCurrentWindow().label;

const call = (request) =>
  invoke("invoke", { request: JSON.stringify(request) }).then(JSON.parse);

const esc = (text) =>
  String(text).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

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
  const [profiles, state] = await Promise.all([
    call({ version: 1, op: "list_profiles", params: {} }),
    call({ version: 1, op: "effective_state", params: {} }),
  ]);
  render(profiles.profiles, state);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") getCurrentWindow().hide();
  });
}

function render(rows, state) {
  surface.innerHTML = `
    <input id="search" type="text" placeholder="Switch profile…" />
    <div id="list"></div>
    <div id="footer">Type to filter · Enter to switch · Esc to close</div>`;
  const search = document.getElementById("search");
  const list = document.getElementById("list");

  const draw = () => {
    const query = search.value.trim().toLowerCase();
    const visible = rows.filter((r) => !query || r.name.toLowerCase().includes(query));
    list.innerHTML = visible
      .map(
        (r, i) => `
      <div class="row${query && i === 0 ? " active" : ""}" data-id="${esc(r.id)}">
        <div class="tile">${esc(r.name.slice(0, 1).toUpperCase())}</div>
        <span class="label">${esc(r.name)}</span>
        <span class="qualifier">${esc(`${r.tools} of ${state.tools.length} tools`)}</span>
        <span class="value">↩</span>
      </div>`
      )
      .join("");
    if (query && !rows.some((r) => r.name.toLowerCase() === query)) {
      list.insertAdjacentHTML(
        "beforeend",
        `<div class="row" data-create="${esc(search.value.trim())}">
           <div class="tile">＋</div>
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
  // exactly that. The shell shows the HUD window; the HUD drives itself from the params.
  const response = await call({ version: 1, op: "switch", params: { profile_id: id } });
  await invoke("show_hud", {
    responseJson: JSON.stringify(response),
    backupId: response.backup ?? "",
  });
  getCurrentWindow().hide();
}

// -- The HUD window --------------------------------------------------------------------

function hud() {
  const response = JSON.parse(new URLSearchParams(location.search).get("response") || "{}");
  const backup = new URLSearchParams(location.search).get("backup");
  const applied = response.outcome === "applied";
  // The design rules settle the default: an unmeasured reload behaviour is "on next launch",
  // in neutral colour — a permanent amber glyph teaches users to ignore amber.
  const result = applied
    ? "on next launch"
    : response.outcome === "rolled back"
      ? "Switch rolled back — restored"
      : "Switch failed";
  surface.className = "hud";
  surface.innerHTML = `
    <span class="result">${esc(result)}</span>
    ${applied && backup ? `<button id="undo">Undo</button>` : ""}`;
  if (applied && backup) {
    document.getElementById("undo").addEventListener("click", async () => {
      await invoke("undo", { backupId: backup });
      document.querySelector(".result").textContent = "Restored";
      document.getElementById("undo").remove();
    });
  }
  setTimeout(() => getCurrentWindow().hide(), 4000);
}
