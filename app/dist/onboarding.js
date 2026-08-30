// Onboarding: two steps, as the catalogue orders them — Tools & providers, then First
// profile — plus the nothing-found state and the Keychain explanation when a key is about to
// travel. Every fact comes through the bridge; the app never probes the machine itself.

import { call, esc, tile, cap, plural } from "./ui.js";

// The platform's own words for the shortcuts — the same fact the panel's footer carries.
const IS_WIN = /Windows NT/.test(navigator.userAgent);

const surface = document.getElementById("surface");

let step = 0; // 0 = found, 1 = first profile, 2 = done
let presence = null;
let harvest = null;
let keys = 0;

export function onboarding(done) {
  load(done);
}

async function load(done) {
  const [state, offer] = await Promise.all([
    call({ version: 1, op: "tool_presence", params: {} }),
    call({ version: 1, op: "harvest", params: {} }),
  ]);
  presence = state.tools;
  harvest = offer;
  surface.className = "window";
  surface.setAttribute("data-tauri-drag-region", "");
  if (!presence.some((t) => t.installed)) {
    renderNone(done);
    return;
  }
  render(done);
}

/// The step indicator, in the titlebar band the prototype puts it in.
function steps() {
  const names = ["Tools & providers", "First profile"];
  return `<div class="titlebar"><div class="ob-steps">${names
    .map(
      (n, i) =>
        `<span class="ob-step${i === step ? " on" : ""}"><span class="n">${i + 1}</span>${n}</span>`
    )
    .join("")}</div></div>`;
}

/// Every screen is `.content` inside the window; `head` is the titlebar band or nothing.
function screen(head, body) {
  surface.innerHTML = `${head}<div class="content">${body}</div>`;
}

function renderNone(done) {
  screen("", `
    <div class="ob-title">No supported tools found</div>
    <div class="ob-sub">tapkey manages Claude Code, Codex and OpenCode</div>
    <div class="group">
      ${presence
        .map(
          (t) => `<div class="row tall">
          ${tile(cap(t.tool))}
          <span class="main"><span class="title">${esc(cap(t.tool))}</span></span>
          <span class="trail"><span class="chip miss">Not installed</span></span>
        </div>`
        )
        .join("")}
    </div>
    <div class="ob-foot"><span></span><button class="btn primary" id="recheck">Check again</button></div>`);
  document.getElementById("recheck").addEventListener("click", () => load(done));
}

function render(done) {
  const inline = harvest.candidates.some((c) => c.credential === "inline");
  screen(steps(), `
    <div class="ob-title">Found on this Mac</div>
    <div class="ob-sub">Current configs are snapshotted before anything changes</div>
    <div class="g-label">Coding tools</div>
    <div class="group">
      ${presence
        .map((t) => {
          const chip = t.installed && t.configured
            ? '<span class="chip ok">Found</span>'
            : t.installed
              ? '<span class="chip">—</span>'
              : '<span class="chip miss">Not installed</span>';
          return `<div class="row tall">
          ${tile(cap(t.tool))}
          <span class="main"><span class="title">${esc(cap(t.tool))}</span></span>
          <span class="trail">${chip}</span>
        </div>`;
        })
        .join("")}
    </div>
    ${
      harvest.candidates.length
        ? `<div class="g-label">Providers in your configs</div>
    <div class="group">
      ${harvest.candidates
        .map(
          (c, i) => `<div class="row tall">
          ${tile(c.id)}
          <span class="main"><span class="title">${esc(c.id)}</span>
            <span class="desc">${esc(cap(c.tool))} · ${
              c.credential === "inline" ? "key copies over" : c.credential === "reference" ? "key stays referenced" : ""
            }</span></span>
          <span class="trail"><label class="swl"><input type="checkbox" checked data-i="${i}"><span class="sw"></span></label></span>
        </div>`
        )
        .join("")}
    </div>
    ${
      inline
        ? `<div class="g-label">Keys go to the Keychain</div>
    <div class="g-desc">tapkey stores provider keys in the macOS Keychain, never in config files. macOS will ask once — the keys stay on this Mac.</div>
    <div class="g-desc">Originals in your configs are left untouched; removing them is a separate step</div>`
        : ""
    }`
        : ""
    }
    <div class="ob-foot">
      <button class="btn" id="ob-later">Set up later</button>
      <button class="btn primary" id="import">Import &amp; continue</button>
    </div>`);

  document.getElementById("ob-later").addEventListener("click", () => later(done));
  document.getElementById("import").addEventListener("click", () => importAll(done));
}

/// The second step: the suggestion the core derived from the live configs becomes the first
/// profile, named and created here — the import that landed a provider but no profile was the
/// live pass's finding, and this step is the catalogue's answer to it.
function renderProfile(done) {
  const suggestion = harvest.suggested_profile;
  screen(steps(), `
    <div class="ob-title">First profile &amp; shortcut</div>
    <div class="ob-sub">Each imported provider is already a profile — pick where to start</div>
    <div class="g-label">All tools</div>
    <div class="group">
      ${
        suggestion
          ? `<div class="row tall">
          ${tile(suggestion.name)}
          <span class="main"><span class="title">${esc(suggestion.name)}</span>
            <span class="desc">${suggestion.tools.map((t) => esc(cap(t.tool))).join(" · ")}</span></span>
          <span class="trail"><span class="chip ok">From your configs</span></span>
        </div>`
          : `<div class="row tall"><span class="main"><span class="title">No suggestion — add a provider in Settings first</span></span></div>`
      }
    </div>
    <div class="g-label">Shortcut</div>
    <div class="group">
      <div class="row tall"><span class="main"><span class="title">Open panel</span></span><span class="trail mono">⌘⇧P</span></div>
      <div class="row tall"><span class="main"><span class="title">Cycle profiles</span></span><span class="trail mono">⌥⌘P</span></div>
    </div>
    <div class="ob-foot">
      <button class="btn" id="ob-back">Back</button>
      <button class="btn primary" id="start">Start using tapkey</button>
    </div>`);

  document.getElementById("ob-back").addEventListener("click", () => {
    step = 0;
    render(done);
  });
  document.getElementById("start").addEventListener("click", () => {
    // The all-set screen paints first and the window leaves after a beat — the person reads
    // what they just gained; the review caught the old order closing the window on a screen
    // nobody ever saw.
    finish();
    setTimeout(done, 2600);
  });
}

async function importAll(done) {
  const boxes = [...document.querySelectorAll("input[data-i]")].filter((b) => b.checked);
  keys = 0;
  for (const box of boxes) {
    const candidate = harvest.candidates[Number(box.dataset.i)];
    const r = await call({
      version: 1, op: "accept_harvest",
      params: { tool: candidate.tool, id: candidate.id },
    });
    if (r.ok && candidate.credential === "inline") keys += 1;
  }
  await createSuggested();
  step = 1;
  renderProfile(done);
}

/// The suggestion's shape into the wire's profile: slots as the map the core expects.
async function createSuggested() {
  const suggestion = harvest.suggested_profile;
  if (!suggestion) return;
  const tools = {};
  for (const t of suggestion.tools) {
    tools[t.tool] = { provider: t.provider, slots: Object.fromEntries(t.slots) };
  }
  const id = suggestion.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "first-profile";
  await call({
    version: 1, op: "create_profile",
    params: { profile: { id, name: suggestion.name, tools } },
  });
}

function later(done) {
  done();
  screen("", `
    <div class="ob-done"><div class="big">—</div>tapkey stays empty until you add a provider.<br />
    <span class="note">Open the panel when you are ready — it will offer to add one</span></div>`);
}

function finish() {
  const copied = keys === 0
    ? ""
    : plural("1 key copied into the Keychain", "{count} keys copied into the Keychain", keys);
  const originals = keys > 0 ? " · Originals left where they were" : "";
  screen("", `
    <div class="ob-done"><div class="big">✓</div>You’re all set — tapkey lives in your menu bar<br />
    <span class="note">${esc(copied)}${esc(originals)}</span>
    <span class="note">${IS_WIN ? "Ctrl+Alt+P opens the panel" : "⌘⇧P opens the panel; ⌥-click the icon returns to the previous profile"}</span>
    <span class="note">Everything else is in Settings</span></div>`);
}
