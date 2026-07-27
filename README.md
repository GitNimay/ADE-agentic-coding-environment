<table>
  <tr>
    <td width="108" height="108" align="center" valign="middle">
      <img src="crates/ade-app/assets/app-icon.png" alt="Termy logo" width="68" height="68" />
    </td>
    <td valign="middle">
      <h2>Termy</h2>
      <p><small><strong>A Windows-first terminal workspace manager, built in Rust.</strong></small></p>
      <p><small>Termy opens real PowerShell and Command Prompt sessions through Windows ConPTY, renders them in a GPU-accelerated desktop window, and keeps project work organized in persistent, folder-backed workspaces.</small></p>
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

**Project workspaces:**  Keep named folders, panes, and working context together.\
**Real Windows terminals:** Run [Windows PowerShell](https://learn.microsoft.com/en-us/powershell/scripting/what-is-windows-powershell), [PowerShell Core](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_powershell_editions), or [Command Prompt](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/cmd) through [ConPTY](https://learn.microsoft.com/en-us/windows/console/pseudoconsoles).\
**Split panes:** Arrange up to six live terminals in one workspace.\
**Session continuity:** Keep shells running after the desktop window closes.\
**Persistent state:** Restore layouts, panes, [Git](https://git-scm.com/docs) context, and recent terminal output.\
**Fast desktop UI:** Render terminal cells through [`eframe`](https://docs.rs/eframe/latest/eframe/), [`egui`](https://docs.rs/egui/latest/egui/), and [`wgpu`](https://docs.rs/wgpu/latest/wgpu/).

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


