// The one kit every surface shares: the bridge call, escaping, the lettermark tile, and the
// tool list — which is core state, taken from effective_state, never invented here twice.

export const call = (request) =>
  window.__TAURI__.core.invoke("invoke", { request: JSON.stringify(request) }).then(JSON.parse);

export const esc = (text) =>
  String(text).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

// The catalogue's plural categories, done as the catalogue demands rather than an English `s`.
export const plural = (one, other, count) => (count === 1 ? one.replace("{count}", "1") : other.replace("{count}", String(count)));

// A recognisable provider renders its own mark — the prototype's symbol library, extracted
// verbatim. Everything else falls back to the lettermark: an unknown brand still deserves a
// face, and inventing one is not ours to do.
const BRANDS = [
  [/z\.ai|^zai|z-ai/i, "i-zai"],
  [/claude|anthropic/i, "i-claude"],
  [/openai|codex|gpt/i, "i-openai"],
  [/opencode/i, "i-opencode"],
  [/deepseek/i, "i-deepseek"],
  [/ollama/i, "i-ollama"],
  [/qwen|dashscope/i, "i-qwen"],
  [/openrouter/i, "i-openrouter"],
  [/^system default$|^tapkey$/i, "i-sys"],
];
export const brandIcon = (name) => BRANDS.find(([re]) => re.test(name))?.[1];

/// The prototype's `ico()`, verbatim in shape: a mark inside a tile, or the lettermark when
/// there is no mark to draw. The classes are the prototype's — `.tile`, `.ic`, `.lettermark`
/// with its `data-l` — because they are what its stylesheet paints.
export const mark = (name, cls = "ic") => {
  const icon = brandIcon(name);
  return icon
    ? `<svg class="${cls}" aria-hidden="true"><use href="#${icon}"/></svg>`
    : `<span class="lettermark" data-l="${esc(String(name).slice(0, 1).toUpperCase())}"></span>`;
};
export const tile = (name) => `<span class="tile">${mark(name)}</span>`;

/// A Segoe Fluent codepoint, the prototype's `fic()`: the win layer swaps these in where macOS
/// draws an SF Symbol, and the css decides which one is visible per platform.
export const fic = (cp, cls) =>
  `<svg class="fic${cls ? ` ${cls}` : ""}" aria-hidden="true"><use href="#f-${cp}"/></svg>`;

export const cap = (s) => s.slice(0, 1).toUpperCase() + s.slice(1);

// The tools list is the core's fact. effective_state names every managed tool; asking the core
// twice for one list is how two callers end up disagreeing about it.
export async function tools() {
  const state = await call({ version: 1, op: "effective_state", params: {} });
  return state.tools.map((t) => t.tool);
}

// A slot's display name, from the catalogue's `prof.slot.*` and `prof.tool.*`. The slot
// inventory itself is the adapter's fact — effective_state names the slots; this only names
// them for a person. The utility slot is the cheap-background-work slot; the catalogue's
// per-tool row label for it is `Background` (grilled, A12) — the same word its auto sentence
// (prof.background.auto) uses.
export const slotName = (slot) => ({
  main: "Main model", utility: "Background", subagent: "Subagent model",
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

// The official-provider presets, from the prototype's stand: a name and a base URL, both
// facts rather than copy. Ollama carries its Cloud/Local option; Custom endpoint is the
// escape hatch for marketplaces and resellers, as the presets note says.
export const PRESETS = [
  { name: "DeepSeek", base_url: "https://api.deepseek.com/v1" },
  { name: "OpenRouter", base_url: "https://openrouter.ai/api/v1" },
  { name: "Z.ai", base_url: "https://api.z.ai/api/anthropic" },
  { name: "Qwen", base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1" },
  { name: "Moonshot Kimi", base_url: "https://api.moonshot.cn/v1" },
  { name: "Ollama", base_url: "http://localhost:11434/v1", opt: "Cloud / Local" },
  { name: "Custom endpoint…" },
];
