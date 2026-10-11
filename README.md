# Rastai Research Studio

An open-source studio for your own neural networks: design them, see inside them, fix them, prove it.

One loop runs through everything: **Design / Import → Observe → Explain → Edit → Train → Verify.**
- Build a model in the Designer or import any Hugging Face model; it runs on Rastai's own inspectable runtime.
- Chat with it, and watch what it represents token by token (logit lens, SAE features, attention, expert routing).
- Steer it, branch it, and pick the better answer; your edits become training data.
- Train a LoRA in seconds, then prove the fix with model tests, an LLM judge, drift and a model diff.

## Status

**Pre-build.** The scope is settled, the riskiest bets have been tested, and the design docs are being written. No product code yet.

| What | Where |
|---|---|
| Product and technical scope (v5.4) | [docs/spec/rastai_spec_v5.md](docs/spec/rastai_spec_v5.md) |
| Spike: measured results for the risky bets | [spike/README.md](spike/README.md) |
| Shared names: components, protocols, commands, events, storage | [docs/architecture/contracts.md](docs/architecture/contracts.md) |
| Code layouts per crate and package | [docs/architecture/layout/](docs/architecture/layout/) |
| Logic flow diagrams | [docs/architecture/flows/](docs/architecture/flows/) |
| Architecture decision records | [docs/adr/](docs/adr/) |
| Implementation plan and backlog | [docs/plan/](docs/plan/) |

## Stack

- **Backend:** Rust (daemon, node, gateway, storage, command layer).
- **Compute:** Python 3.12 + PyTorch (in-house model runtime, trainer, interpretability, RL), run as jobs under the node.
- **UI:** plain JavaScript + React (JSX, short JSDoc types; no TypeScript), in a Tauri 2 shell or any browser.
- **Storage:** SQLite on every machine. Optionally, your own MySQL or MariaDB server as a long-term archive (you run it; Rastai never bundles it).
- **Platforms:** Linux with NVIDIA, AMD (ROCm) or CPU first; macOS and Windows later.

## License

Apache-2.0 ([LICENSE](LICENSE)). Contributions are accepted under the Developer Certificate of Origin; see [CONTRIBUTING.md](CONTRIBUTING.md).
