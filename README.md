<table>
  <tr>
    <td width="132" align="center">
      <img src="crates/ade-app/assets/app-icon.png" alt="Termy logo" width="76" />
    </td>
    <td>
      <h1>Termy</h1>
      <p><strong>A Windows-first terminal workspace manager, built in Rust.</strong></p>
      <p>Termy opens real PowerShell and Command Prompt sessions through Windows ConPTY, renders them in a GPU-accelerated desktop window, and keeps project work organized in persistent, folder-backed workspaces.</p>
      <p>
        <img alt="Rust" src="https://img.shields.io/badge/Rust-000000?style=flat-square&logo=rust&logoColor=white" />
        <img alt="eframe" src="https://img.shields.io/badge/eframe-1F2937?style=flat-square&logo=egui&logoColor=white" />
        <img alt="egui" src="https://img.shields.io/badge/egui-374151?style=flat-square&logo=egui&logoColor=white" />
        <img alt="wgpu" src="https://img.shields.io/badge/wgpu-2563EB?style=flat-square&logoColor=white" />
        <img alt="Windows ConPTY" src="https://img.shields.io/badge/Windows_ConPTY-0078D4?style=flat-square&logo=windows11&logoColor=white" />
        <img alt="SQLite" src="https://img.shields.io/badge/SQLite-003B57?style=flat-square&logo=sqlite&logoColor=white" />
      </p>
    </td>
  </tr>
</table>

## Demo

<a href="docs/assets/termy-demo.mp4">
  <img src="docs/assets/termy-demo.gif" alt="Termy demo video preview" width="760" />
</a>

## Features

<dl>
  <dt><strong>Project workspaces</strong></dt>
  <dd>Keep named folders, panes, and working context together.</dd>

  <dt><strong>Real Windows terminals</strong></dt>
  <dd>Run <a href="https://learn.microsoft.com/en-us/powershell/scripting/what-is-windows-powershell">Windows PowerShell</a>, <a href="https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_powershell_editions">PowerShell Core</a>, or <a href="https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/cmd">Command Prompt</a> through <a href="https://learn.microsoft.com/en-us/windows/console/pseudoconsoles">ConPTY</a>.</dd>

  <dt><strong>Split panes</strong></dt>
  <dd>Arrange up to six live terminals in one workspace.</dd>

  <dt><strong>Session continuity</strong></dt>
  <dd>Keep shells running after the desktop window closes.</dd>

  <dt><strong>Persistent state</strong></dt>
  <dd>Restore layouts, panes, <a href="https://git-scm.com/docs">Git</a> context, and recent terminal output.</dd>

  <dt><strong>Fast desktop UI</strong></dt>
  <dd>Render terminal cells through <a href="https://docs.rs/eframe/latest/eframe/"><code>eframe</code></a>, <a href="https://docs.rs/egui/latest/egui/"><code>egui</code></a>, and <a href="https://docs.rs/wgpu/latest/wgpu/"><code>wgpu</code></a>.</dd>
</dl>


## Installation

### Download

1. Download the latest Windows build: [windows-x64-termy.exe](https://github.com/GitNimay/ADE-agentic-coding-environment/releases/latest/download/windows-x64-termy.exe).
2. Move the executable to a permanent folder.
3. Run `windows-x64-termy.exe`.

### Manual Installation

Clone the repository, build the desktop app, then run it locally:

```powershell
git clone https://github.com/GitNimay/ADE-agentic-coding-environment.git
cd ADE-agentic-coding-environment
cargo build -p ade-app
.\target\debug\ade-app.exe
```

## Quick summary

Explore project capabilities in the [feature guide](docs/features.md).
Review system design in the [architecture notes](docs/architecture.md).

Follow the [release process](docs/releasing.md) to publish updates.
Keep releases clear and consistent.

<sub>Licensed under **MIT.** **Built by [Nimesh](https://www.n1m35h.in/).**</sub>


