<table>
  <tr>
    <td width="132" align="center">
      <img src="crates/ade-app/assets/app-icon.png" alt="Termy logo" width="76" />
    </td>
    <td>
      <h1>Termy</h1>
      <p><strong>A Windows-first terminal workspace manager, built in Rust.</strong></p>
      <p>Termy opens real PowerShell and Command Prompt sessions through Windows ConPTY, renders them in a GPU-accelerated desktop window, and keeps project work organized in persistent, folder-backed workspaces.</p>
    </td>
  </tr>
</table>

<p>
  <a href="https://github.com/GitNimay/ADE-agentic-coding-environment/releases/latest/download/windows-x64-termy.exe">
    <img alt="Download for Windows" src="https://img.shields.io/badge/Download-Windows_x64-111111?style=for-the-badge&logo=windows11&logoColor=white" />
  </a>
  <img alt="Rust" src="https://img.shields.io/badge/Rust-1.95%2B-B7410E?style=for-the-badge&logo=rust&logoColor=white" />
  <img alt="Platform" src="https://img.shields.io/badge/Platform-Windows_11-0A66C2?style=for-the-badge&logo=windows11&logoColor=white" />
</p>

## Demo

<video src="docs/assets/termy-demo.mp4" controls width="760"></video>

<p><sub>A concise walkthrough of Termy's workspace flow, split panes, and persistent terminal context.</sub></p>

If your Markdown viewer does not render inline video, open [the demo file](docs/assets/termy-demo.mp4) directly.

## Features

- **Workspace-first terminal control:** Create named workspaces from real project folders, switch between them quickly, and keep each workspace tied to its own pane layout and working directory.
- **Native Windows terminal sessions:** Run PowerShell, PowerShell Core, or Command Prompt through Windows ConPTY with terminal input, output, resizing, selection, copy, paste, and bracketed paste support.
- **Managed split panes:** Work with up to six terminal panes per workspace in balanced layouts that remain readable as panes are opened, closed, focused, and resized.
- **Background session continuity:** Terminal processes are owned by a per-user daemon, so closing the desktop window does not intentionally stop running shells.
- **Persistent project state:** Layouts, workspace metadata, active panes, terminal dimensions, scrollback replay, clipboard image paths, and Git context are restored through SQLite-backed state.
- **Command-aware interface:** PowerShell prompt hooks surface command blocks, current working directory, recent command suggestions, and Git status without blocking the UI thread.
- **Focused desktop experience:** A GPU-rendered `eframe` interface uses bundled Geist fonts, a compact command palette, accessible controls, and responsive sidebar behavior for narrow and wide windows.

## Tech Stack

<p>
  <img alt="Rust" src="https://img.shields.io/badge/Rust-000000?style=flat-square&logo=rust&logoColor=white" />
  <img alt="eframe" src="https://img.shields.io/badge/eframe-1F2937?style=flat-square&logo=egui&logoColor=white" />
  <img alt="egui" src="https://img.shields.io/badge/egui-374151?style=flat-square&logo=egui&logoColor=white" />
  <img alt="wgpu" src="https://img.shields.io/badge/wgpu-2563EB?style=flat-square&logoColor=white" />
  <img alt="Windows ConPTY" src="https://img.shields.io/badge/Windows_ConPTY-0078D4?style=flat-square&logo=windows11&logoColor=white" />
  <img alt="VT100" src="https://img.shields.io/badge/VT100-0F766E?style=flat-square&logoColor=white" />
  <img alt="SQLite" src="https://img.shields.io/badge/SQLite-003B57?style=flat-square&logo=sqlite&logoColor=white" />
  <img alt="AccessKit" src="https://img.shields.io/badge/AccessKit-4B5563?style=flat-square&logoColor=white" />
</p>

## Installation

### Download

1. Download the latest Windows build: [windows-x64-termy.exe](https://github.com/GitNimay/ADE-agentic-coding-environment/releases/latest/download/windows-x64-termy.exe).
2. Move the executable to a permanent folder.
3. Run `windows-x64-termy.exe`.

Termy currently targets Windows 11 x64. The standalone build is unsigned, so Windows SmartScreen may show an unknown publisher warning.

### Manual Installation

Clone the repository, build the desktop app, then run it locally:

```powershell
git clone https://github.com/GitNimay/ADE-agentic-coding-environment.git
cd ADE-agentic-coding-environment
cargo build -p ade-app
.\target\debug\ade-app.exe
```

Requirements:

- Windows 11 x64
- Rust 1.95 or newer
- Git

## Keyboard Shortcuts

| Category | Shortcut | Action |
| --- | --- | --- |
| Command Palette | `Ctrl+Shift+P` | Open the command palette |
| Command Palette | `Ctrl+K` | Open the command palette |
| Workspace Management | `Ctrl+Shift+N` | Choose a folder and create a workspace |
| Pane Management | `Ctrl+Shift+D` | Split pane right |
| Pane Management | `Ctrl+Shift+E` | Split pane down |
| Pane Management | `Ctrl+Shift+W` | Close the active pane |
| Pane Management | `Ctrl+Alt+Left` or `Ctrl+Alt+Up` | Move focus to the previous pane |
| Pane Management | `Ctrl+Alt+Right` or `Ctrl+Alt+Down` | Move focus to the next pane |
| Workspace Navigation | `Ctrl+PageDown` | Move to the next workspace |
| Workspace Navigation | `Ctrl+PageUp` | Move to the previous workspace |
| Workspace Navigation | `F2` | Rename the active workspace |
| Terminal Focus | `Ctrl+Shift+Right` | Focus terminal right |
| Terminal Focus | `Ctrl+Shift+Left` | Focus terminal left |
| Terminal Focus | `Ctrl+Shift+Down` | Focus terminal down |
| Terminal Focus | `Ctrl+Shift+Up` | Focus terminal up |
| Terminal | `Ctrl+C` | Copy selected text |
| Terminal | `Ctrl+Shift+C` | Copy selected text |
| Terminal | `Ctrl+V` | Paste text |
| Terminal | `Ctrl+Shift+V` | Paste a clipboard image |
| Terminal | `Shift+Insert` | Paste text |

## Documentation

- [Feature guide](docs/features.md)
- [Architecture notes](docs/architecture.md)
- [Release process](docs/releasing.md)

---

<sub>Licensed under MIT OR Apache-2.0. Built by [Nimesh](https://www.n1m35h.in/)🤍.</sub>
