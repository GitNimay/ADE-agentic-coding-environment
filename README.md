# ADE

A Windows-first terminal workspace manager built in Rust.

The application opens real PowerShell/Command Prompt sessions through Windows ConPTY and provides
a workspace sidebar, GPU-rendered terminal panes, persistent split layouts, scrollback, clipboard
support, and keyboard-driven commands. A per-user background daemon keeps terminals alive while
the window is closed and stores workspace state in SQLite.

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

Build an unsigned MSIX with `powershell -ExecutionPolicy Bypass -File packaging\build-msix.ps1`.
Production packages must be signed with a certificate matching the manifest publisher.
