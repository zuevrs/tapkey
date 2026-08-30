// The sheets: Effective state and History, opened where the person thinks of them. Both are
// reads through the bridge — the chains and the moments are the core's to give, and this
// surface renders what it is told. Everything composes from the row primitive and the tokens.

import { call, esc, tile, cap, plural } from "./ui.js";

const surface = document.getElementById("surface");

export function effectiveState() {
  surface.className = "sheet-page";
  drawEffective();
}

async function drawEffective() {
  const state = await call({ version: 1, op: "effective_state", params: {} });
  surface.innerHTML = `
    <header class="sheet-head"><h2>What is in effect right now</h2></header>
    ${state.tools
      .map(
        (tool) => `
      <section class="card">
        <div class="row">
          ${tile(cap(tool.tool))}
          <span class="label">${esc(cap(tool.tool))}</span>
          <span class="value">${esc(tool.endpoint.effective ?? "— not managed")}</span>
        </div>
        ${tool.slots
          .filter((slot) => slot.effective !== null || slot.owned)
          .map(
            (slot) => `
          <div class="row">
            <span class="label">${esc(slotName(slot.slot))}</span>
            <span class="qualifier">${slot.drifted ? "changed outside tapkey" : slot.owned ? "" : "not managed"}</span>
            <span class="value">${esc(slot.effective ?? "—")}</span>
          </div>`
          )
          .join("")}
        ${tool.attentions?.length ? attentions(tool.attentions) : ""}
      </section>`
      )
      .join("")}
    <p class="note">Chains name every place that had an opinion</p>`;
}

function attentions(list) {
  return `<div class="row attn"><span class="note">${list
    .map((a) => esc(attentionText(a)))
    .join("<br/>")}</span></div>`;
}

function attentionText(a) {
  switch (a.kind) {
    case "tool_will_not_start": return `${a.key} in ${a.file} stops this tool starting`;
    case "slot_provider_ignored": return `${a.key} kept the tool's endpoint — one endpoint serves every slot`;
    case "format_not_served": return "this provider has no API this tool speaks";
    case "format_untested": return "switched to a provider nothing has tested";
    default: return a.kind;
  }
}

function slotName(slot) {
  const names = {
    main: "Main model", utility: "Utility model", subagent: "Subagent model",
    review: "Review model", effort: "Effort level", verbosity: "Verbosity",
    opus: "Opus pin", sonnet: "Sonnet pin", fable: "Fable pin",
    advisor: "Advisor model", fallback: "Fallback model",
  };
  return names[slot] ?? cap(slot);
}

// -- History ---------------------------------------------------------------------------

export function history() {
  surface.className = "sheet-page";
  drawHistory();
}

async function drawHistory() {
  const { entries } = await call({ version: 1, op: "list_history", params: {} });
  if (!entries.length) {
    surface.innerHTML = `
      <header class="sheet-head"><h2>Switch history</h2></header>
      <p class="note">Nothing switched yet</p>`;
    return;
  }
  surface.innerHTML = `
    <header class="sheet-head"><h2>Switch history</h2>
      <p class="note">Last 50 switches are restorable</p></header>
    ${entries
      .map(
        (e) => `
      <div class="card">
        <div class="row">
          ${tile(e.name)}
          <span class="label">${esc(e.name)}</span>
          <span class="qualifier">${esc(when(e.instant))}</span>
          <button class="act" data-id="${esc(e.id)}" data-kind="${esc(e.kind)}"
                  data-name="${esc(e.name)}" ${e.restorable ? "" : "disabled"}>
            ${e.restorable ? "Restore" : "Can’t be restored"}</button>
        </div>
        <div class="row"><span class="qualifier">${esc(plural("1 file", "{count} files", e.files))}</span></div>
        <div class="result note" role="status"></div>
      </div>`
      )
      .join("")}`;

  surface.querySelectorAll("button[data-id]").forEach((button) =>
    button.addEventListener("click", async () => {
      const result = button.closest(".card").querySelector(".result");
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
