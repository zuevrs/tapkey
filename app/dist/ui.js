// The one kit every surface shares: the bridge call, escaping, the lettermark tile, and the
// tool list — which is core state, taken from effective_state, never invented here twice.

export const call = (request) =>
  window.__TAURI__.core.invoke("invoke", { request: JSON.stringify(request) }).then(JSON.parse);

export const esc = (text) =>
  String(text).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

// The catalogue's plural categories, done as the catalogue demands rather than an English `s`.
export const plural = (one, other, count) => (count === 1 ? one.replace("{count}", "1") : other.replace("{count}", String(count)));

export const tile = (name) =>
  `<div class="tile" data-l="${esc(name.slice(0, 1).toUpperCase())}"></div>`;

export const cap = (s) => s.slice(0, 1).toUpperCase() + s.slice(1);

// The tools list is the core's fact. effective_state names every managed tool; asking the core
// twice for one list is how two callers end up disagreeing about it.
export async function tools() {
  const state = await call({ version: 1, op: "effective_state", params: {} });
  return state.tools.map((t) => t.tool);
}

// A slot's display name, from the catalogue's `prof.slot.*`. The slot inventory itself is the
// adapter's fact — effective_state names the slots; this only names them for a person.
export const slotName = (slot) => ({
  main: "Main model", utility: "Utility model", subagent: "Subagent model",
  review: "Review model", effort: "Effort level", verbosity: "Verbosity",
  opus: "Opus pin", sonnet: "Sonnet pin", fable: "Fable pin",
  advisor: "Advisor model", fallback: "Fallback model",
}[slot] ?? cap(slot));

// The catalogue's per-slot hints, shown under their own row only.
export const slotHint = (slot) => ({
  utility: "Titles, summaries, compaction",
  subagent: "Overrides every subagent, without editing their files",
  fallback: "Used when the main model is overloaded",
  aliases: "What /model sonnet resolves to here",
}[slot] ?? "");
