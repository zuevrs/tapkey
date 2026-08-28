# tapkey

Switch which AI provider every coding tool on your machine talks to, from the menu bar.

Claude Code, Codex and OpenCode each keep their provider settings in a different file, in a different format, in a different place. Moving all of them from an official API to DeepSeek, OpenRouter, Z.ai, a local model or a corporate gateway means editing several files by hand and getting the syntax right every time.

tapkey edits those files for you and then reads them back, so what it shows is what the tool will actually use — not what tapkey meant to write. It changes only the keys that select a provider or a model, leaves the rest of each file untouched, and keeps a copy of everything as it was before the first change.

## Status

Early development. There is nothing to install yet: the engine is being built first, with tests, and the apps follow.

## Requirements

macOS 14 or later. Windows support comes after macOS.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Security reports: [SECURITY.md](SECURITY.md).

## License

MIT — see [LICENSE](LICENSE) and [NOTICE](NOTICE).
