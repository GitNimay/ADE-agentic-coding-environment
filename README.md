<img src="crates/ade-app/assets/app-icon.png" alt="Termy logo" width="64" />

# Termy

**A Windows-first terminal workspace manager built in Rust.**

[Download for Windows](https://github.com/GitNimay/ADE-agentic-coding-environment/releases/latest/download/windows-x64-termy.exe) · [Watch Demo](https://res.cloudinary.com/dvzxfbcsd/video/upload/v1785139663/lxb6rwhhikj12e6ux3xu.mp4) · [View License](LICENSE)

## Demo

[![Termy demo](https://res.cloudinary.com/dvzxfbcsd/video/upload/so_1,w_1400,c_limit,q_auto,f_jpg/v1785139663/lxb6rwhhikj12e6ux3xu.jpg)](https://res.cloudinary.com/dvzxfbcsd/video/upload/v1785139663/lxb6rwhhikj12e6ux3xu.mp4)

<sub>Click the preview to watch the full demo.</sub>

## Overview

Termy opens real PowerShell and Command Prompt sessions through Windows ConPTY and renders them in a GPU-accelerated desktop window.

Organize your work into named, folder-backed workspaces, split up to six terminal panes per workspace, and keep sessions alive in a background daemon even after the window is closed.

Layouts, scrollback, clipboard history, and Git context persist across restarts through SQLite.

## Features

* Real PowerShell and Command Prompt sessions through Windows ConPTY
* Named workspaces backed by local folders
* Up to six terminal panes in each workspace
* Horizontal and vertical pane splitting
* Background daemon for persistent terminal sessions
* GPU-accelerated terminal rendering
* Persistent layouts and terminal scrollback
* Clipboard history with text and image support
* Persistent Git context
* Automatic background update checks
* Local SQLite-based storage

## Install

Download [`windows-x64-termy.exe`](https://github.com/GitNimay/ADE-agentic-coding-environment/releases/latest/download/windows-x64-termy.exe), move it to a permanent folder, and run it.

**Requirements:** Windows 11 x64

> [!NOTE]
> The standalone build is currently unsigned, so Windows SmartScreen may display an unknown publisher warning.

Official builds check for updates in the background and offer a silent restart when the application is idle.

## Keyboard Shortcuts

| Shortcut                        | Action                   |
| :------------------------------ | :----------------------- |
| `Ctrl+Shift+N`                  | New workspace            |
| `Ctrl+Shift+D`                  | Split pane right         |
| `Ctrl+Shift+E`                  | Split pane down          |
| `Ctrl+Shift+W`                  | Close active pane        |
| `Ctrl+Alt+Arrow`                | Move pane focus          |
| `Ctrl+C` / `Ctrl+Shift+C`       | Copy selected text       |
| `Ctrl+V` / `Ctrl+Shift+V`       | Paste text or image      |
| `Ctrl+Shift+P`                  | Open the command palette |
| `Ctrl+PageUp` / `Ctrl+PageDown` | Switch workspace         |
| `F2`                            | Rename workspace         |

## Technology

![Rust](https://img.shields.io/badge/Rust-000000?style=flat-square\&logo=rust\&logoColor=white)
![eframe](https://img.shields.io/badge/eframe-252525?style=flat-square\&logo=rust\&logoColor=white)
![wgpu](https://img.shields.io/badge/wgpu-252525?style=flat-square\&logo=webgpu\&logoColor=white)
![Windows ConPTY](https://img.shields.io/badge/Windows_ConPTY-252525?style=flat-square\&logo=windows11\&logoColor=white)
![SQLite](https://img.shields.io/badge/SQLite-252525?style=flat-square\&logo=sqlite\&logoColor=white)
![VT100](https://img.shields.io/badge/VT100-252525?style=flat-square\&logo=gnometerminal\&logoColor=white)

| Area               | Technology     |
| :----------------- | :------------- |
| Language           | Rust           |
| Desktop framework  | eframe         |
| GPU rendering      | wgpu           |
| Terminal backend   | Windows ConPTY |
| Terminal emulation | VT100          |
| Persistence        | SQLite         |

---

<sub>Licensed under <a href="LICENSE">MIT OR Apache-2.0</a> · Built by <strong><a href="https://www.n1m35h.in/">Nimesh</a></strong></sub>
