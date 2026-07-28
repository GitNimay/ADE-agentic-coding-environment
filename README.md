<a href="https://github.com/GitNimay/ADE-agentic-coding-environment">
  <img align="left" src="./crates/ade-app/assets/app-icon.png" alt="Termy logo" width="76" />
</a>
<p>
  <span style="font-size: 1.5em;"><strong>Termy ::</strong></span><br>
  <strong>A focused Windows terminal workspace, engineered in <a href="https://www.rust-lang.org/">Rust</a>.</strong> Run <a href="https://learn.microsoft.com/en-us/powershell/scripting/what-is-windows-powershell">Windows PowerShell</a> and <a href="https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/cmd">Command Prompt</a> through <a href="https://learn.microsoft.com/en-us/windows/console/pseudoconsoles">ConPTY</a> with persistent project workspaces, split panes, restored layouts, and recent terminal context.
  <br />
  Termy uses <a href="https://docs.rs/eframe/latest/eframe/">eframe</a>, <a href="https://docs.rs/egui/latest/egui/">egui</a>, <a href="https://docs.rs/wgpu/latest/wgpu/">wgpu</a>, and SQLite-backed state to keep desktop terminal sessions fast, local, and easy to resume. Start with the <a href="docs/features.md">feature guide</a> or review the <a href="docs/architecture.md">architecture notes</a>.
</p>

<p>
  <img alt="Rust" src="https://img.shields.io/badge/Rust-000000?style=flat-square&logo=rust&logoColor=white" />
  <img alt="eframe" src="https://img.shields.io/badge/eframe-1F2937?style=flat-square&logo=egui&logoColor=white" />
  <img alt="egui" src="https://img.shields.io/badge/egui-374151?style=flat-square&logo=egui&logoColor=white" />
  <img alt="wgpu" src="https://img.shields.io/badge/wgpu-2563EB?style=flat-square&logoColor=white" />
  <img alt="Windows ConPTY" src="https://img.shields.io/badge/Windows_ConPTY-0078D4?style=flat-square&logo=windows11&logoColor=white" />
  <img alt="SQLite" src="https://img.shields.io/badge/SQLite-003B57?style=flat-square&logo=sqlite&logoColor=white" />
</p>

## <sub>Demo</sub>

<p align="center">
  <a href="docs/assets/termy-demo.mp4">
    <img src="docs/assets/termy-demo.gif" alt="Termy demo video preview" width="680" />
  </a>
</p>

## <sub>Features</sub>

<sub><strong>Project workspaces:</strong> Keep named folders, panes, and working context together.</sub>\
<sub><strong>Real Windows terminals:</strong> Run <a href="https://learn.microsoft.com/en-us/powershell/scripting/what-is-windows-powershell">Windows PowerShell</a>, <a href="https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_powershell_editions">PowerShell Core</a>, or <a href="https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/cmd">Command Prompt</a> through <a href="https://learn.microsoft.com/en-us/windows/console/pseudoconsoles">ConPTY</a>.</sub>\
<sub><strong>Split panes:</strong> Arrange up to six live terminals in one workspace.</sub>\
<sub><strong>Session continuity:</strong> Keep shells running after the desktop window closes.</sub>\
<sub><strong>Persistent state:</strong> Restore layouts, panes, <a href="https://git-scm.com/docs">Git</a> context, and recent terminal output.</sub>\
<sub><strong>Fast desktop UI:</strong> Render terminal cells through <code>eframe</code>, <code>egui</code>, and <code>wgpu</code>.</sub>

## <sub>Installation</sub>

### <sub>Download</sub>

1. <sub>Download the latest Windows build: <a href="https://github.com/GitNimay/ADE-agentic-coding-environment/releases/latest/download/windows-x64-termy.exe">windows-x64-termy.exe</a>.</sub>
2. <sub>Move the executable to a permanent folder.</sub>
3. <sub>Run <code>windows-x64-termy.exe</code>.</sub>

### <sub>Manual Installation</sub>

<sub>Clone the repository, build the desktop app, then run it locally:</sub>

```powershell
git clone https://github.com/GitNimay/ADE-agentic-coding-environment.git
cd ADE-agentic-coding-environment
cargo build -p ade-app
.\target\debug\ade-app.exe
```

## <sub>Quick summary</sub>

<sub>Explore project capabilities in the <a href="docs/features.md">feature guide</a>.</sub>
<sub>Review system design in the <a href="docs/architecture.md">architecture notes</a>.</sub>

<sub>Follow the <a href="docs/releasing.md">release process</a> to publish updates.</sub>
<sub>Keep releases clear and consistent.</sub>

<sub>Licensed under <strong>MIT.</strong> <strong>Built by <a href="https://www.n1m35h.in/">Nimesh</a>.</strong></sub>
