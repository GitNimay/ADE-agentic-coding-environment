<img src="crates/ade-app/assets/app-icon.png" alt="Termy" width="64" />

# Termy

**A Windows-first terminal workspace manager, built in Rust.**

Termy opens real PowerShell and Command Prompt sessions through Windows ConPTY and renders them in a GPU-accelerated desktop window. Organize work into named, folder-backed workspaces, split up to six terminal panes per workspace, and keep sessions alive in a background daemon even after the window is closed. Layouts, scrollback, clipboard history, and Git context all persist across restarts through SQLite.

### Demo

[![Watch the demo](https://img.shields.io/badge/▶-Watch_the_demo-181717?style=for-the-badge)](https://res.cloudinary.com/dvzxfbcsd/video/upload/v1785139663/lxb6rwhhikj12e6ux3xu.mp4)

<sub>GitHub strips `&lt;video&gt;` tags that point to external hosts, which is why the clip wasn't showing. For a real inline player, drag the mp4 directly into the README editor on github.com — GitHub will host it and generate a tag that renders — then swap it in for the badge above.</sub>

## Features

- **Workspaces** — name and pin folder-backed workspaces you can jump between instantly
- **Split panes** — up to six ConPTY sessions per workspace, arranged however you like
- **Background daemon** — sessions keep running even after you close the window
- **Persistent state** — layouts, scrollback, clipboard history, and Git context survive restarts via SQLite
- **GPU rendering** — smooth, low-latency terminal output through wgpu

## Tech Stack

| Layer | Technology |
| --- | --- |
| Language | ![Rust](https://img.shields.io/badge/Rust-000000?style=flat-square&logo=rust&logoColor=white) |
| UI / Rendering | ![eframe](https://img.shields.io/badge/eframe-FF6B35?style=flat-square&logo=egui&logoColor=white) ![wgpu](https://img.shields.io/badge/wgpu-4DAA57?style=flat-square&logoColor=white) |
| Terminal | ![ConPTY](https://img.shields.io/badge/Windows_ConPTY-7B68EE?style=flat-square&logoColor=white) ![VT100](https://img.shields.io/badge/VT100-20B2AA?style=flat-square&logoColor=white) |
| Storage | ![SQLite](https://img.shields.io/badge/SQLite-003B57?style=flat-square&logo=sqlite&logoColor=white) |

## Install

Download [`windows-x64-termy.exe`](https://github.com/GitNimay/ADE-agentic-coding-environment/releases/latest/download/windows-x64-termy.exe) to a permanent folder and run it.

**Requirements:** Windows 11 x64

The standalone build is currently unsigned, so Windows SmartScreen may show an unknown publisher warning. Official builds check for updates in the background and offer a silent restart when idle.

## Keyboard Shortcuts

| Shortcut | Action | Shortcut | Action |
| --- | --- | --- | --- |
| `Ctrl+Shift+N` | New workspace | `Ctrl+C` / `Ctrl+Shift+C` | Copy selected text |
| `Ctrl+Shift+D` | Split pane right | `Ctrl+V` / `Ctrl+Shift+V` | Paste text or image |
| `Ctrl+Shift+E` | Split pane down | `Ctrl+Shift+P` | Command palette |
| `Ctrl+Shift+W` | Close active pane | `Ctrl+PageUp` / `Ctrl+PageDown` | Switch workspace |
| `Ctrl+Alt+Arrow` | Move pane focus | `F2` | Rename workspace |

---

<sub>Licensed under [MIT OR Apache-2.0](LICENSE) · Built by [Nimesh](https://www.n1m35h.in/)</sub>
