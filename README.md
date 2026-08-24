# tapkey

Switch which AI provider every coding tool on your Mac talks to, from the menu bar.

Claude Code, Codex and OpenCode each keep their provider settings in a different file, in a different format, in a different place. Moving all of them from an official API to DeepSeek, OpenRouter, Z.ai, a local model or a corporate gateway means editing several files by hand and getting the syntax right every time. tapkey does it in one click, and shows you what is actually in effect afterwards.

**Status: in design.** There is nothing to install yet, and no code in this repository.

## Planned

- One panel, from the menu bar icon or a global hotkey: pick a profile, every managed tool moves.
- Pin a single tool to its own provider when one job needs something different.
- Keys live in the macOS Keychain, and reach tools indirectly wherever the tool supports it.
- Every switch is atomic and reversible: undo from the confirmation, or restore an earlier one.
- Balances and spend for providers that publish them.
- An inspector that answers the only question that matters when something looks wrong: what is this tool actually using, and which file decided that.

## Not planned

Routing your traffic. tapkey rewrites config files and steps aside — no daemon, no proxy, nothing between your tools and their providers. If you want account pools and failover, run a proxy and add it to tapkey as another provider.

## Requirements

macOS 26 or later.

## License

MIT — see [LICENSE](LICENSE) and [NOTICE](NOTICE).
