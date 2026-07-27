<div align="center">

<img src="crates/ade-app/assets/app-icon.png" alt="termy" width="120" />

# termy

A Windows-first terminal workspace manager built in Rust.

</div>

---

<video src="https://res.cloudinary.com/dvzxfbcsd/video/upload/v1785139663/lxb6rwhhikj12e6ux3xu.mp4" controls muted autoplay loop playsinline width="100%"></video>

---

Termy opens real PowerShell and Command Prompt sessions through Windows ConPTY and renders them in a GPU-accelerated desktop window. Organize work into named, folder-backed workspaces, split up to six terminal panes per workspace, and keep sessions alive in a background daemon even after the window is closed. Layouts, scrollback, clipboard history, and Git context persist across restarts through SQLite.

<div align="center">

![Rust](https://img.shields.io/badge/Rust-000000?style=flat&logo=rust&logoColor=white)
![eframe](https://img.shields.io/badge/eframe-FF6B35?style=flat&logo=egui&logoColor=white)
![wgpu](https://img.shields.io/badge/wgpu-4DAA57?style=flat&logoColor=white)
![ConPTY](https://img.shields.io/badge/ConPTY-7B68EE?style=flat&label=Windows+ConPTY&logoColor=white)
![SQLite](https://img.shields.io/badge/SQLite-003B57?style=flat&logo=sqlite&logoColor=white)
![VT100](https://img.shields.io/badge/VT100-20B2AA?style=flat&logoColor=white)

</div>

## Install

Download [`windows-x64-termy.exe`](https://github.com/GitNimay/ADE-agentic-coding-environment/releases/latest/download/windows-x64-termy.exe) to a permanent folder and run it.

**Requirements:** Windows 11 x64

The standalone build is currently unsigned, so Windows SmartScreen may show an unknown publisher warning. Official builds check for updates in the background and offer a silent restart when idle.

## Keyboard Shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+Shift+N` | New workspace |
| `Ctrl+Shift+D` | Split pane right |
| `Ctrl+Shift+E` | Split pane down |
| `Ctrl+Shift+W` | Close active pane |
| `Ctrl+Alt+Arrow` | Move pane focus |
| `Ctrl+C` / `Ctrl+Shift+C` | Copy selected text |
| `Ctrl+V` / `Ctrl+Shift+V` | Paste text or image |
| `Ctrl+Shift+P` | Command palette |
| `Ctrl+PageUp` / `Ctrl+PageDown` | Switch workspace |
| `F2` | Rename workspace |

---

<div align="center">

<img src="crates/ade-app/assets/app-icon.png" alt="termy" width="40" />

Licensed under [MIT OR Apache-2.0](LICENSE)

Built by **[Nimesh](https://www.n1m35h.in/)**

</div>
