# tapkey

Switch which AI provider every coding tool on your Mac talks to, from the menu bar.

Claude Code, Codex and OpenCode each keep their provider settings in a different file, in a different format, in a different place. Moving all of them from an official API to DeepSeek, OpenRouter, Z.ai, a local model or a corporate gateway means editing several files by hand and getting the syntax right every time. tapkey does it in one click, and shows you what is actually in effect afterwards.

**Status: early development.** There is nothing to install yet. The engine is being built first, with tests; the app follows.

## Planned

- Adding a provider takes four fields — name, base URL, key, Test. Which API formats it speaks, which models it
  serves and what context each one holds are worked out from there, not asked for.
- One panel, from the menu bar icon or a global hotkey: pick a profile, every managed tool moves.
- Every model setting each tool has, not just the main one: the utility model behind titles and summaries, the
  aliases `/model sonnet` resolves through, the subagent and advisor models, a fallback, the effort level. Pick a
  provider and they are all filled in; open the slots and set any of them yourself.
- Pin a single tool to its own provider when one job needs something different.
- Keys live in the macOS Keychain and reach Claude Code and Codex indirectly, through a helper command they run
  themselves. OpenCode has no such mechanism, so its key is written to a file with owner-only permissions, and the
  app says so on that provider instead of implying secrecy it cannot deliver.
- Every switch is atomic and reversible: undo from the confirmation, or restore an earlier one.
- Balances and spend for providers that publish them.
- An inspector that answers the only question that matters when something looks wrong: what is this tool actually using, and which file decided that.

## Not planned

**Everything in those files that is not about providers and models.** Permissions, MCP servers, hooks, agent
definitions and instruction files stay yours to edit; tapkey has no opinion about them and will not touch them.
It also does not rewrite the agent definitions in `.claude/agents` or `.opencode/agents` — those hold prompts and
permissions, and the model a subagent runs on can be set without opening them.

**Project configs.** A `settings.json` or `opencode.json` committed in a repository outranks anything tapkey writes
for you. It reports which file won and which key did it, and leaves the repository alone.

**Routing your traffic.** tapkey rewrites config files and steps aside — no daemon, no proxy, nothing between your tools and their providers. If you want account pools and failover, run a proxy and add it to tapkey as another provider.

## Requirements

macOS 14 or later. Windows support is planned; the engine is built and tested for Linux from the
first commit so that portability is compiled rather than assumed.

## License

MIT — see [LICENSE](LICENSE) and [NOTICE](NOTICE).
