# termy

A Windows-first terminal workspace manager built in Rust.

The application opens real PowerShell/Command Prompt sessions through Windows ConPTY and provides
a workspace sidebar, GPU-rendered terminal panes, persistent split layouts, scrollback, clipboard
support, and keyboard-driven commands. A per-user background daemon keeps terminals alive while
the window is closed and stores workspace state in SQLite.

## Install

Download
[`termy.exe`](https://github.com/GitNimay/ADE-agentic-coding-environment/releases/latest/download/termy.exe)
to a permanent folder and run it. Windows 11 x64 is required.

The standalone build is free and currently unsigned, so Windows SmartScreen may show an unknown
publisher warning. GitHub records build-provenance attestations for release executables. Official
builds check the latest GitHub Release in the background, replace the executable when a newer
version is available, and ask for a convenient restart. Workspace data remains in the user's local
application-data directory.

Each workspace supports up to six terminals. Layouts are managed by terminal count: two or three
terminals form one row, four form a 2x2 grid, five use rows of three and two, and six use a 3x2
grid. Dividers remain resizable and the layout compacts automatically when a terminal closes.

## Development

Requirements:

- Windows 11 x64
- Rust 1.95 using the MSVC target
- Visual Studio Build Tools and a Windows SDK

Run the checks:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run the desktop app:

```powershell
cargo run --release -p ade-app
```

Inspect or automate the daemon:

```powershell
cargo run -p ade-cli -- list
cargo run -p ade-cli -- new D:\code\project project
cargo run -p ade-cli -- exec git status
cargo run -p ade-cli -- shutdown
```

Default shortcuts:

- `Ctrl+Shift+N`: new workspace
- `Ctrl+Shift+D`: split right
- `Ctrl+Shift+E`: split down
- `Ctrl+Alt+Arrow`: move pane focus
- `Ctrl+Shift+W`: close active pane
- `Ctrl+Shift+C`: copy the selected terminal text
- `Ctrl+Shift+V`: paste with bracketed-paste support
- `Ctrl+Shift+P`: command palette
- `Ctrl+PageUp` / `Ctrl+PageDown`: switch workspace
- `F2`: rename the active workspace

An optional unsigned development MSIX can be built with `packaging\build-msix.ps1`. Maintainer
release instructions are in [`docs/releasing.md`](docs/releasing.md).
