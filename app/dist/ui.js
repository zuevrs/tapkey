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
