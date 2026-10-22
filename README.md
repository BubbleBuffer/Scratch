# Scratch

How I always wanted to organise my life: notes, tasks, plans and project work in one place, local-first, with a hub I host myself and AI tools as first-class users.

> **Pre-alpha. Not ready for real use.** Scratch is unstable and changes often. Vault, sync and hub data formats may change without a migration path, so keep your own backups and expect to start over. There are no support, security or compatibility promises, and issues or pull requests may go unanswered.

- **Pages and blocks.** Everything is a page made of blocks, kept as plain files on your device. No raw syntax to write.
- **Databases.** Pages with properties, collected into databases with views.
- **Your own hub.** Devices sync through a small server you run yourself. See [Running your own hub](wiki/architecture/hub.md).
- **Agent tab.** A built-in agent that reads and edits the vault with you.
- **MCP access.** Claude, ChatGPT and other AI tools use the vault over MCP through the hub.
- **Windows, Android and a CLI.** A Windows app with an installer, an Android app, and the `notes` command line tool.

How it works now: the [project wiki](wiki/index.md).

## Build

You need stable Rust, Flutter 3.47.5 and `flutter_rust_bridge_codegen` 2.13.0. Windows builds need Visual Studio 2022 with ATL; Android builds need NDK 27.3.13750724 and JDK 17. The Rust workspace fetches its agent crates from [BubblePaw-Core](https://github.com/BubbleBuffer/BubblePaw-Core) on GitHub.

```sh
cargo install flutter_rust_bridge_codegen --version 2.13.0 --locked
cargo test --workspace --locked --exclude notes-tauri    # Rust tests

cd app && flutter pub get
flutter build windows --release                          # Windows app
flutter build apk --release                              # Android (debug-signed without a release keystore)

cargo build --release --locked -p vault_tools --bin notes   # notes CLI
cargo build --release --locked -p notes_hub                 # hub server (notes-hub)
```

The Windows installer is described in [Windows installer](wiki/architecture/windows-install.md).

## Licence

`AGPL-3.0-or-later`; see [LICENSE](LICENSE). Bundled third-party components and their licences are listed in [THIRD_PARTY.md](THIRD_PARTY.md).

Scratch is made by [BubbleBuffer](https://0xbubbuff.de).
