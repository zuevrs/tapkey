// The sheets: Effective state and History, opened where the person thinks of them. Both are
// reads through the bridge — the chains and the moments are the core's to give, and this
// surface renders what it is told. Everything composes from the row primitive and the tokens.

import { call, esc, mark, tile, cap, plural, slotName } from "./ui.js";

const surface = document.getElementById("surface");

// Esc closes the sheet, as it closes the panel — the one gesture that must work everywhere
// a window can disappear.
const { getCurrentWindow } = window.__TAURI__.window;
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") getCurrentWindow().hide();
});

export function effectiveState() {
  surface.className = "sheet";
  drawEffective();
}

async function drawEffective() {
  const state = await call({ version: 1, op: "effective_state", params: {} });
  // The prototype's shape: a lede counting the tools in effect, then one card per tool with
  // its own head and badge, the rows inside it, and the source line under them.
  const managed = state.tools.filter((t) => t.endpoint.effective);
  const off = state.tools.filter((t) => !t.endpoint.effective);
  surface.innerHTML = `
    <h4>What is in effect right now</h4>
    <div class="eff-lede">${managed.length} of ${state.tools.length} tools are managed by tapkey${
      off.length ? ` · ${off.map((t) => esc(cap(t.tool))).join(", ")} is not` : ""
    }</div>
    ${state.tools
      .map(
        (tool) => `
      <section class="card eff-card open">
        <div class="card-head">
          ${mark(tool.tool)}${esc(cap(tool.tool))}
          <span class="badge2 ${tool.endpoint.effective ? "is-live" : "is-next"}">${
            tool.endpoint.effective ? "In effect" : "Not in effect"
          }</span>
        </div>
        ${tool.slots
          .filter((slot) => slot.effective !== null || slot.owned)
          .map(
            (slot) => `
          <div class="row${slot.drifted ? " bad" : ""}">
            <span class="main"><span class="title">${esc(slotName(slot.slot))}</span>
              ${slot.drifted
                ? `<span class="desc">${esc(cap(tool.tool))} — changed outside tapkey</span>`
                : slot.owned ? "" : `<span class="desc">not managed</span>`}</span>
            <span class="trail"><span class="val mono">${esc(slot.effective ?? "—")}</span></span>
          </div>`
          )
          .join("")}
        <div class="card-note">${
          tool.endpoint.effective
            ? `Endpoint <span class="mono">${esc(tool.endpoint.effective)}</span>`
            : "Endpoint — not managed"
        }</div>
        ${tool.attentions?.length ? attentions(tool, tool.attentions) : ""}
      </section>`
      )
      .join("")}
    <div class="card-note">Chains name every place that had an opinion</div>
    <div class="sheet-foot">
      <button class="btn sm" id="refresh">Refresh</button>
      <button class="btn primary" id="done">Done</button>
    </div>`;
  document.getElementById("done")?.addEventListener("click", () => getCurrentWindow().hide());
  document.getElementById("refresh")?.addEventListener("click", drawEffective);
}

function attentions(tool, list) {
  // The prototype's attention block, the panel's own shape reused inside a card.
  return list
    .map(
      (a) => `<div class="p-attn" style="margin:8px"><span>⚠ ${esc(
        attentionText(cap(tool.tool), a)
      )}</span></div>`
    )
    .join("");
}

/// An attention renders the catalogue's own sentence with its named placeholders filled —
/// a paraphrase is how the interface starts disagreeing with the catalogue.
function attentionText(tool, a) {
  switch (a.kind) {
    case "tool_will_not_start":
      return `${tool} will not start — \`${a.key}\` in ${a.file} stops it`;
    case "slot_provider_ignored":
      return `${tool} put ${a.key} on another provider — it has one endpoint for every slot`;
    case "format_not_served":
      return `${tool} stayed on its current provider — the provider has no API this tool speaks`;
    case "format_untested":
      return `${tool} switched to a provider nothing has tested`;
    default:
      return a.kind;
  }
}

// -- History ---------------------------------------------------------------------------

export function history() {
  surface.className = "sheet";
  drawHistory();
}

async function drawHistory() {
  const { entries } = await call({ version: 1, op: "list_history", params: {} });
  if (!entries.length) {
    surface.innerHTML = `
      <h4>Switch history</h4>
      <div class="card-note">Nothing switched yet</div>
      <div class="sheet-foot"><button class="btn primary" id="done">Done</button></div>`;
    document.getElementById("done").addEventListener("click", () => getCurrentWindow().hide());
    return;
  }
  // The prototype's history: one group of tall rows, each a mark, a name, the instant and the
  // file count as its description, and Restore in the trail — and the sheet-foot with Done, the
  // close the prototype's sheet carries (the native close is the OS's; Done is the sheet's).
  surface.innerHTML = `
    <h4>Switch history</h4>
    <div class="eff-lede">Last 50 switches are restorable</div>
    <div class="group">
      ${entries
        .map(
          (e) => `
        <div class="row tall${e.kind === "snapshot" ? " snap" : ""}">
          ${tile(e.name)}
          <span class="main"><span class="title">${esc(e.name)}</span>
            <span class="desc">${esc(when(e.instant))} · ${esc(
              plural("1 file", "{count} files", e.files)
            )}</span>
            <span class="desc" role="status"></span></span>
          <span class="trail"><button class="btn sm" data-id="${esc(e.id)}" data-kind="${esc(e.kind)}"
                  data-name="${esc(e.name)}" ${e.restorable ? "" : "disabled"}>
            ${e.restorable ? "Restore" : "Can’t be restored"}</button></span>
        </div>`
        )
        .join("")}
    </div>
    <div class="sheet-foot"><button class="btn primary" id="done">Done</button></div>`;

  document.getElementById("done").addEventListener("click", () => getCurrentWindow().hide());

  surface.querySelectorAll("button[data-id]").forEach((button) =>
    button.addEventListener("click", async () => {
      const result = button.closest(".row").querySelector('[role="status"]');
      // The envelope is the one shape the core knows; a restore names its target.
      const target = button.dataset.kind === "snapshot"
        ? "snapshot"
        : { target: "backup", id: button.dataset.id };
      const r = await call({ version: 1, op: "restore", params: { target } });
      result.textContent =
        r.outcome === "applied"
          ? button.dataset.kind === "snapshot"
            ? "Restored the first-run snapshot"
            : `Restored ${button.dataset.name}`
          : r.failure
            ? `${r.failure.kind} — nothing was changed`
            : "";
      if (r.outcome === "applied") button.disabled = true;
    })
  );
}

function when(instant) {
  // Instants are UTC ISO-ish from the store; the platform formats dates, we only make them
  // readable enough to tell apart. Rendered local, labelled honestly.
  const date = new Date(instant);
  return Number.isNaN(date.getTime()) ? instant : date.toLocaleString();
}
