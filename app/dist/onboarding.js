// Onboarding: one screen, three sections. The harvest offer is the engine; this surface offers
// and never adopts (core ticket 30's rule), the import creates profiles from what the person
// ticked, and the done section says what the first switch will snapshot.

import { call, esc, tile, cap, plural } from "./ui.js";

export function onboarding(done) {
  draw(done);
}

async function draw(done) {
  const [presence, harvest] = await Promise.all([
    call({ version: 1, op: "tool_presence", params: {} }),
    call({ version: 1, op: "harvest", params: {} }),
  ]);

  const surface = document.getElementById("surface");
  surface.className = "onboarding";
  surface.innerHTML = `
    <h2>Welcome to tapkey</h2>

    <h3>Tools &amp; providers</h3>
    <p class="note">Current configs are snapshotted before anything changes</p>
    <div class="chips">
      ${presence.tools
        .map((t) => {
          const chip = !t.installed ? "Not installed" : t.configured ? "Found" : "—";
          return `<span class="chip ${t.installed && t.configured ? "ok" : ""}">
            ${esc(cap(t.tool))} <b>${esc(chip)}</b></span>`;
        })
        .join("")}
    </div>

    <div class="candidates">
      ${harvest.candidates.length ? "<h3>Providers in your configs</h3><p class='note'>Keys go to the Keychain; these files stay untouched</p>" : ""}
      ${harvest.candidates
        .map(
          (c, i) => `
        <label class="row ${c.declined ? "declined" : ""}">
          <input type="checkbox" data-i="${i}" ${c.declined ? "" : "checked"} />
          ${tile(c.id)}
          <span class="label">${esc(c.id)}</span>
          <span class="qualifier">${esc(cap(c.tool))}</span>
          <span class="value">${esc(c.credential === "inline" ? "key copies over" : c.credential === "reference" ? "key stays referenced" : "")}</span>
        </label>`
        )
        .join("")}
    </div>

    ${
      harvest.candidates.length === 0
        ? `<h3>No supported tools found</h3>
           <p class="note">tapkey manages Claude Code, Codex and OpenCode</p>`
        : ""
    }

    <div class="actions">
      <button class="act" id="later">Set up later</button>
      <button class="act primary" id="import">Import &amp; continue</button>
    </div>`;

  surface.querySelector("#later").addEventListener("click", () => {
    // The catalogue's honest consequence, said on the screen before the flag is set.
    surface.innerHTML = `
      <h2>Welcome to tapkey</h2>
      <p class="note">tapkey stays empty until you add a provider</p>
      <div class="actions"><button class="act primary" id="close">OK</button></div>`;
    surface.querySelector("#close").addEventListener("click", done);
  });
  surface.querySelector("#import").addEventListener("click", () => importAll(harvest, done));
}

async function importAll(harvest, done) {
  const boxes = [...document.querySelectorAll("input[data-i]")].filter((b) => b.checked);
  let keys = 0;
  for (const box of boxes) {
    const candidate = harvest.candidates[Number(box.dataset.i)];
    const r = await call({
      version: 1, op: "accept_harvest",
      params: { tool: candidate.tool, id: candidate.id },
    });
    if (r.ok && candidate.credential === "inline") keys += 1;
  }
  finish(keys, done);
}

function finish(keys, done) {
  const surface = document.getElementById("surface");
  // The originals sentence is earned by the count: some keys travelled and were copied out of
  // files that still hold them. None travelled — nothing to say about originals.
  const copied = keys === 0
    ? ""
    : `${plural("1 key copied into the Keychain", "{count} keys copied into the Keychain", keys)}`;
  const originals = keys > 0 ? " · Originals left where they were" : "";
  surface.innerHTML = `
    <h2>You’re all set — tapkey lives in your menu bar</h2>
    <p class="note">${esc(copied)}${esc(originals)}</p>
    <p class="note">⌘⇧P opens the panel; ⌥-click the icon returns to the previous profile</p>
    <p class="note">Everything else is in Settings</p>
    <div class="actions"><button class="act primary" id="start">Start using tapkey</button></div>`;
  surface.querySelector("#start").addEventListener("click", done);
}
