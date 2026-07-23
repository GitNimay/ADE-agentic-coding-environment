# termy Feature Guide

This document describes the features that are currently implemented in **termy**, how a user
interacts with them, and how the application makes them work. It reflects the code in this
repository rather than planned functionality.

## Contents

- [1. Application overview](#1-application-overview)
- [2. Workspaces](#2-workspaces)
- [3. Terminal panes and layouts](#3-terminal-panes-and-layouts)
- [4. Terminal interaction](#4-terminal-interaction)
- [5. Command blocks, working directory, and Git context](#5-command-blocks-working-directory-and-git-context)
- [6. Command palette](#6-command-palette)
- [7. Navigation and keyboard shortcuts](#7-navigation-and-keyboard-shortcuts)
- [8. Responsive desktop interface](#8-responsive-desktop-interface)
- [9. Persistent background sessions](#9-persistent-background-sessions)
- [10. State persistence and restoration](#10-state-persistence-and-restoration)
- [11. Shell selection and PowerShell enhancements](#11-shell-selection-and-powershell-enhancements)
- [12. CLI automation](#12-cli-automation)
- [13. Error handling and safeguards](#13-error-handling-and-safeguards)
- [14. Installation and updates](#14-installation-and-updates)
- [15. Feature architecture](#15-feature-architecture)
- [16. Current boundaries and limitations](#16-current-boundaries-and-limitations)

## 1. Application overview

termy is a Windows-first terminal workspace manager. It combines multiple real Windows terminal
sessions in a single GPU-rendered desktop window and organizes those sessions into named,
folder-backed workspaces.

The application is split into two long-running roles:

```text
Desktop UI (ade-app.exe)
        |
        | per-user Windows named pipe
        v
Background daemon (ade-app.exe --daemon)
        |
        +-- ConPTY session for pane 1
        +-- ConPTY session for pane 2
        +-- ...
```

The desktop UI renders workspaces, layouts, terminal cells, dialogs, and commands. The daemon owns
the shell processes and their pseudoconsoles. This separation allows terminal processes to remain
alive after the desktop window is closed.

The main window uses a custom dark interface, a borderless title bar, bundled Geist fonts, and the
`wgpu` renderer supplied through `eframe`. It opens maximized by default, starts at a logical size
of 1280 x 800, and enforces a minimum window size of 480 x 360.

## 2. Workspaces

A workspace groups a project folder, a name, a pane layout, and the currently active pane. Every
workspace and pane has a stable UUID so the daemon can persist and synchronize them without
depending on display order or names.

### 2.1 Automatic first workspace

On first launch, if the daemon has no saved workspaces, the UI creates **Workspace 1** using the
application's current directory. A new workspace starts with one terminal pane.

### 2.2 Create a workspace

A workspace can be created with:

- the **+** button in the workspace sidebar or compact workspace bar;
- **Ctrl+Shift+N**; or
- **New Workspace** in the command palette.

The app opens a native folder picker. The chosen folder becomes the workspace root and the
initial shell working directory. The folder's final path component becomes the workspace name; if
that component is unavailable, the app falls back to a numbered name such as `Workspace 2`.

Creating a workspace also:

1. creates its first pane at the default terminal size of 80 columns by 24 rows;
2. starts a shell in the selected folder;
3. makes the new workspace active; and
4. persists and publishes the updated application snapshot.

### 2.3 Workspace sidebar

At normal window widths, the left sidebar lists each workspace with:

- a stable colored identity tile derived from its UUID;
- the first alphanumeric character of its name;
- the workspace name;
- a compact form of its root path; and
- a green status dot when at least one pane is starting or running.

Hovering a workspace shows its full name and root path. Clicking selects it. Double-clicking opens
the rename dialog. Right-clicking opens a context menu with **Rename** and **Delete** actions.

The sidebar normally appears as a narrow icon rail. Hovering the rail or the left-edge trigger for
about 180 ms expands it. It remains expanded while a context menu is open and collapses roughly
450 ms after the pointer leaves.

### 2.4 Rename a workspace

Rename is available from:

- **F2** for the active workspace;
- a double-click on a workspace item;
- the workspace context menu; or
- **Rename Workspace...** in the command palette.

The rename dialog accepts **Enter** to save and **Escape** or **Cancel** to discard. Names are
trimmed before submission, and an empty name is rejected. Renaming changes only the label; it does
not rename or move the backing folder.

### 2.5 Switch workspaces

A workspace can be selected by clicking it in the sidebar or compact menu. Keyboard users can use
**Ctrl+PageDown** for the next workspace and **Ctrl+PageUp** for the previous workspace. Switching
wraps around at either end of the list.

The active workspace ID is stored by the daemon, so all connected control surfaces see the same
authoritative selection.

### 2.6 Close a workspace

**Delete** in the workspace context menu or **Close Workspace** in the command palette removes the
workspace and closes every terminal session it owns. If the closed workspace was active, the
daemon selects a neighboring remaining workspace when possible.

This operation is immediate and currently has no confirmation dialog or in-app undo.

## 3. Terminal panes and layouts

Each pane is an independent Windows ConPTY session. It has its own shell process tree, terminal
screen, scrollback, current working directory, status, input channel, and resizable character grid.

### 3.1 Add or split a pane

The active workspace can add a terminal with:

- **Ctrl+Shift+D** or **Split Pane Right**;
- **Ctrl+Shift+E** or **Split Pane Down**; or
- a click in the center of an empty workspace.

If the workspace has no panes, clicking its empty state creates one. Otherwise, a split request
creates a new pane based on the active pane. The new pane inherits the target pane's current
working directory and most recent terminal dimensions, then becomes active.

The `Right` and `Down` commands are exposed as distinct user actions, but the daemon currently
normalizes every pane-count change into the managed grid described below. The requested direction
does not permanently determine the resulting grid.

### 3.2 Managed layouts for one to six panes

Workspaces support at most six panes. Whenever a pane is created or closed, the daemon rebuilds a
balanced layout from the remaining pane order:

| Pane count | Arrangement |
| ---: | --- |
| 0 | Empty workspace |
| 1 | One full pane |
| 2 | One row of two equal panes |
| 3 | One row of three equal panes |
| 4 | Two rows of two panes |
| 5 | Three panes on the first row, two on the second |
| 6 | Two rows of three panes |

The layout is stored as a recursive split tree. Column splits divide left and right areas; row
splits divide top and bottom areas. Split ratios are constrained to the range 0.1 through 0.9 and
the daemon validates that an updated layout contains exactly the pane IDs owned by that workspace.

### 3.3 Resize panes

Dragging a divider resizes the adjacent layout branches. The divider turns blue while hovered or
dragged. Minimum visual pane sizes are 220 points wide and 120 points high, including allowances
for nested panes and internal dividers.

Temporary window constraints do not overwrite the user's saved proportions. Once a drag changes a
ratio, the UI sends the complete validated layout to the daemon, which persists it. Terminal rows
and columns are recalculated from the pane's pixel dimensions and forwarded to ConPTY.

### 3.4 Focus a pane

Clicking or starting a text selection inside a pane makes it active. The active pane receives
keyboard input, displays its cursor, and shows a blue accent beside the current command block.

Pane focus can also move in layout order with:

- **Ctrl+Alt+Left** or **Ctrl+Alt+Up** for the previous pane; and
- **Ctrl+Alt+Right** or **Ctrl+Alt+Down** for the next pane.

Focus navigation wraps at the first and last panes.

### 3.5 Close a pane

**Ctrl+Shift+W** or **Close Active Pane** starts a short close animation, then asks the daemon to
close the active pane. The remaining panes are compacted into a new managed grid. Closing the last
pane leaves the workspace intact in an empty state, from which a new terminal can be opened by
clicking the workspace.

If a shell process exits on its own, the daemon reports its exit status and automatically removes
that pane from the workspace.

### 3.6 Six-terminal safeguard

At six panes, another split is blocked. The UI displays a **Terminal Limit Reached** modal
explaining that one terminal must be closed first. The daemon independently enforces the same
limit, so CLI or protocol clients cannot bypass it.

## 4. Terminal interaction

### 4.1 Real shell execution through ConPTY

Input is written to the pane's host-side ConPTY pipe and terminal output is read on an independent
worker. Separate input and output workers prevent one blocked direction from stopping the other.
Each pane owns a separate pseudoconsole and process tree.

Synchronized-output frames (`CSI ? 2026 h` through `CSI ? 2026 l`) are committed atomically, with
a bounded timeout for malformed or interrupted streams. This prevents menu redraws from exposing
half-rendered frames when their bytes arrive in separate pipe reads.

The renderer parses output with `vt100` and draws the terminal cell by cell. It supports:

- ANSI indexed colors and RGB colors;
- foreground and background colors;
- inverse and dim text;
- italics and underlining;
- wide-character continuation cells;
- the terminal cursor and alternate screen state; and
- application cursor-key mode.

### 4.2 Keyboard input encoding

Ordinary text is encoded as UTF-8 and sent to the active pane. Alt-modified text receives an
Escape prefix. The terminal encoder also supports:

- Enter, Tab, Shift+Tab, Backspace, and Escape;
- arrow, Home, End, Insert, Delete, Page Up, and Page Down keys;
- F1 through F12;
- Ctrl+A through Ctrl+Z control bytes when Shift is not held;
- application-mode arrow sequences; and
- modifier-aware arrow sequences for Shift, Alt, and Ctrl.

Application-level shortcuts are consumed before terminal input so actions such as splitting,
copying, and opening the palette do not leak their keystrokes into the shell.

### 4.3 Selection and copy

Dragging over terminal cells creates a rectangular start/end selection that is normalized even
when dragged backwards. Selected cells are highlighted in blue. **Ctrl+Shift+C** copies the
selected text to the system clipboard. Clicking without dragging clears the current selection.

Invisible command-block marker text inserted by the PowerShell prompt hook is removed from copied
content.

### 4.4 Paste and bracketed paste

**Ctrl+Shift+V** reads from the Windows clipboard. Plain text is normalized by converting Windows
CRLF line endings to LF and removing null characters before it is sent. Clipboard images are saved
as PNG files under `%LOCALAPPDATA%\ADE\clipboard-images`, then the quoted absolute image path is
pasted into the active terminal for CLI apps that accept image/file paths. If the active terminal
has enabled bracketed-paste mode, the app wraps the payload in the standard `ESC[200~` and
`ESC[201~` sequences so compatible shells and programs can treat it as pasted data.

Paste events delivered directly by the UI follow the same normalization and bracketed-paste logic.
Clipboard access failures appear in the in-app error dialog.

### 4.5 Scrollback

Each UI pane maintains up to 10,000 parsed scrollback lines. Scrolling the pointer wheel over a
pane changes its scrollback position. When the main screen is at the bottom, output is visually
bottom-anchored by its last rendered row rather than by the application cursor, so moving through
a selection menu cannot move the viewport. Alternate-screen programs and scrolled-back views are
rendered from the top. Application terminal modes also suppress shell block decorations so a TUI
owns its complete grid.

### 4.6 Automatic terminal sizing

The available content rectangle is divided by the measured monospace cell dimensions to calculate
the largest complete grid. Zero-sized grids are avoided. A size change updates both the local
terminal parser and the daemon's ConPTY session, and the dimensions are included in persisted pane
metadata.

## 5. Command blocks, working directory, and Git context

PowerShell panes use a custom prompt hook that emits hidden markers around prompts. The UI detects
those markers and renders command history as visually separated blocks instead of showing the
marker text.

### 5.1 Command-block presentation

Completed commands are separated with thin horizontal rules. The current command area is docked
near the bottom of the pane and receives a small blue accent when active. New and closing panes use
short reveal and close animations to make layout changes easier to follow.

### 5.2 Working-directory tracking

Before each PowerShell prompt, the hook emits an OSC 7 file URI for the current provider path. The
daemon parses OSC 7 sequences even when they are split across output reads, updates the pane's
current working directory, saves it, and publishes the new snapshot to the UI.

The current directory is shown as a compact folder chip in the active command header. Long paths
are shortened to fit the available space.

Protocol clients can also explicitly submit a `ReportCwd` request, although the desktop UI relies
on the PowerShell OSC 7 hook.

### 5.3 Git status display

The UI checks the current directory for Git information asynchronously about every 1.5 seconds.
When the directory is in a Git repository, the command header can show:

- the current branch, or a short commit ID in detached HEAD state;
- the number of changed and untracked files;
- total added lines; and
- total deleted lines.

The implementation uses `git status --porcelain=v1 --branch --untracked-files=normal`,
`git diff --numstat HEAD`, and `git rev-parse --short HEAD`. The status worker is kept off the UI
thread. Header elements progressively hide or shorten when a pane is too narrow.

Git statistics represent the working-tree diff against `HEAD`; repositories without a usable Git
command or status response simply omit the Git header.

## 6. Command palette

The command palette opens with **Ctrl+Shift+P** or **Ctrl+K**. It appears over a dimmed backdrop and
provides a case-insensitive substring search. Leading and trailing query whitespace is ignored.

Commands are grouped into **Actions** and **Navigation**:

- New Workspace
- Split Pane Right
- Split Pane Down
- Close Active Pane
- Rename Workspace...
- Close Workspace
- Next Workspace
- Previous Workspace

Use **Up** and **Down** to move through filtered results, **Enter** to run the selected command, and
**Escape** or a click on the backdrop to close the palette. Selection wraps at both ends, moves to
a hovered row, and resets to the first result whenever the query changes.

Terminal input is temporarily disabled while the palette or rename dialog is active.

## 7. Navigation and keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+Shift+N` | Choose a folder and create a workspace |
| `Ctrl+Shift+D` | Add a pane using the Split Right action |
| `Ctrl+Shift+E` | Add a pane using the Split Down action |
| `Ctrl+Shift+W` | Close the active pane |
| `Ctrl+Shift+C` | Copy selected terminal text |
| `Ctrl+Shift+V` | Paste into the active terminal |
| `Ctrl+Shift+P` | Open the command palette |
| `Ctrl+K` | Open the command palette |
| `Ctrl+Alt+Left` / `Ctrl+Alt+Up` | Focus the previous pane |
| `Ctrl+Alt+Right` / `Ctrl+Alt+Down` | Focus the next pane |
| `Ctrl+PageUp` | Focus the previous workspace |
| `Ctrl+PageDown` | Focus the next workspace |
| `F2` | Rename the active workspace |

The bottom footer of each terminal displays `Ctrl+Shift+P` as a reminder for the palette. Its text
is brighter on the active pane.

## 8. Responsive desktop interface

### 8.1 Custom window controls

The borderless window implements its own minimize, maximize/restore, and close buttons. The title
bar can be dragged to move the window and double-clicked to toggle maximization. The close button
uses a distinct red hover state.

Closing the window closes only the UI process; it does not intentionally shut down the background
daemon or its terminal sessions.

### 8.2 Sidebar breakpoints

The workspace interface adapts to available width:

- above 960 points, the expanded sidebar is 256 points wide;
- from 600 through 960 points, it uses a 224-point tablet width;
- while collapsed, the sidebar is a 56-point identity rail; and
- at 600 points or less, it becomes a 40-point top bar with a workspace drop-down.

The compact top bar keeps workspace selection, context menus, and new-workspace creation available
on narrow windows.

### 8.3 Accessibility integration

The `eframe` build enables AccessKit, and interactive custom elements publish widget roles and
labels for window controls, workspace items, command rows, and buttons. Keyboard focus and hover
states are visually represented throughout the interface.

## 9. Persistent background sessions

### 9.1 Automatic daemon startup

When the desktop app starts, it first attempts to open the current user's named pipe. If no daemon
is available, it launches the same executable with `--daemon` as a detached, hidden process and
retries the connection for up to roughly six seconds.

A per-user Windows mutex prevents multiple daemon instances. The pipe and mutex names sanitize the
Windows username so unsupported characters cannot alter the object name.

### 9.2 Sessions survive UI closure

The daemon, not the UI, owns every ConPTY and shell process. Closing and reopening the window
therefore leaves running commands and interactive sessions alive. On reattach, the UI receives the
authoritative workspace snapshot followed by buffered terminal output, then resumes live output.

### 9.3 Bounded output replay

The daemon retains the newest 8 MiB of raw terminal output per pane. If the UI is disconnected,
output continues to accumulate within that bound. On attach, the daemon replays each non-empty
buffer so the newly created terminal parser can reconstruct recent state.

Only the most recently attached desktop-style subscriber receives live broadcast events. Snapshot
requests from utility clients remain available independently.

### 9.4 Status and process lifecycle

Pane status progresses through `Starting`, `Running`, `Exited { exit_code }`, or
`FailedToStart { message }`. Input and resize operations are sent only to starting or running
sessions. The daemon polls each root process for exit, publishes status changes, and removes exited
panes from the persisted layout.

## 10. State persistence and restoration

The daemon stores one versioned application snapshot in SQLite at:

```text
%LOCALAPPDATA%\ADE\ade.db
```

The snapshot contains:

- active workspace ID;
- workspace IDs, names, root folders, layouts, and active panes; and
- pane IDs, workspace ownership, status, working directory, process label, columns, and rows.

Updates are serialized as JSON and atomically replaced inside a SQLite transaction. Schema and
snapshot versions are tracked so unsupported saved formats are rejected instead of silently
misread.

### What is restored

After the daemon process itself restarts, it loads the saved workspaces and creates a fresh shell
for every persisted pane using the stored directory and terminal size. Persisted pane statuses are
reset to `Starting` before those shells are launched.

### What is not restored

A machine restart or explicit daemon shutdown cannot restore the memory of an arbitrary running
process, its exact interactive application state, or its full prior terminal output. Those
processes are recreated as fresh shells. Live-process continuity applies while the daemon remains
running, such as when only the desktop UI is closed.

## 11. Shell selection and PowerShell enhancements

For each new session, the daemon selects the first available shell in this order:

1. `pwsh.exe` found on `PATH`;
2. `powershell.exe` found on `PATH`;
3. the executable in `%COMSPEC%`; or
4. `C:\Windows\System32\cmd.exe` as the final fallback.

PowerShell and PowerShell Core are started with `-NoExit -Command` and a prompt hook. The hook:

- imports PSReadLine when available;
- enables inline history prediction when supported;
- maps Right Arrow to accept a suggestion;
- emits OSC 7 working-directory updates;
- emits hidden command-block divider markers; and
- renders a colored `❯` prompt.

Errors while enabling optional PSReadLine behavior are deliberately ignored so the terminal still
opens. Command Prompt fallback sessions remain functional terminals, but they do not receive the
PowerShell-specific prompt, automatic OSC 7 reporting, or command-block markers.

## 12. CLI automation

`ade-cli` connects to an already running daemon through the same per-user pipe. It does not start
the daemon automatically.

### `list`

```powershell
cargo run -p ade-cli -- list
```

Requests the current authoritative snapshot and prints it as pretty JSON. Running the CLI with no
subcommand also defaults to `list`.

### `new [path] [name]`

```powershell
cargo run -p ade-cli -- new D:\code\project project
```

Creates a workspace. The path defaults to the CLI's current directory. The name defaults to the
last path component or `Workspace`.

### `exec <command>`

```powershell
cargo run -p ade-cli -- exec git status
```

Finds the active workspace and active pane, joins the remaining arguments with spaces, and writes
the command followed by carriage return to that pane. This injects text as if it were typed; it
does not create a separate process outside the terminal.

### `shutdown`

```powershell
cargo run -p ade-cli -- shutdown
```

Requests an orderly daemon shutdown. The daemon stops its sessions, cancels connected pipe I/O,
wakes its accept loop, and exits. Because the daemon owns the ConPTY sessions, this also ends the
live terminals.

The CLI verifies the protocol version on responses and reports unknown commands, missing `exec`
input, or the absence of an active workspace/pane.

## 13. Error handling and safeguards

The application has validation at both UI and daemon boundaries:

- workspace names cannot be blank;
- terminal dimensions must be non-zero and fit the Windows `COORD` range;
- split ratios must be finite and between 0.1 and 0.9;
- layouts cannot contain duplicate or foreign pane IDs;
- each workspace is capped at six panes;
- IPC frames are capped at 16 MiB and rejected before oversized allocation;
- clients and daemon must use protocol version 1;
- only one daemon is allowed per Windows user; and
- input is ignored for sessions that are no longer live.

Daemon and protocol failures are delivered as `Error` events and shown in a dismissible **termy
error** dialog. UI connection diagnostics are appended to:

```text
%LOCALAPPDATA%\ADE\ade-ui.log
```

PTY startup failures become pane status updates rather than crashing the whole daemon. When a
persisted working directory no longer exists, the daemon starts that pane in its own current
directory instead.

## 14. Installation and updates

Every successful CI run caused by a push to `main` publishes `termy.exe` directly in a GitHub
Release. Official builds check that release feed in a background thread at startup. When a newer
semantic version exists, the executable is replaced in place without interrupting live terminals;
the UI asks the user to restart when convenient. Update failures are recorded in the diagnostic
log and do not prevent startup. The supported target is Windows 11 x64.

The repository also contains `packaging/build-msix.ps1` for producing an unsigned development
MSIX. That package is for local testing and is not part of public releases. See
[releasing.md](releasing.md) for the automated release workflow.

## 15. Feature architecture

| Crate | Responsibility |
| --- | --- |
| `ade-app` | Desktop UI, terminal parser and rendering, interaction, clipboard, Git status, daemon client |
| `ade-daemon` | Authoritative state, terminal lifetime, replay buffers, named-pipe server, persistence orchestration |
| `ade-core` | Workspace model, stable IDs, session statuses, split-tree validation, managed layouts |
| `ade-pty` | Windows ConPTY creation, process startup, synchronous pipe I/O, resize, wait, handle cleanup |
| `ade-protocol` | Versioned requests/events, snapshots, framed JSON transport, per-user pipe naming |
| `ade-storage` | SQLite schema, migration record, atomic snapshot save/load |
| `ade-cli` | Read-only inspection and basic daemon automation from the command line |

### Request and update flow

Most mutating actions follow this sequence:

```text
User action
  -> ade-app sends a versioned ClientRequest
  -> ade-daemon validates and mutates authoritative state
  -> daemon performs the runtime side effect (spawn, close, input, or resize)
  -> daemon persists when required
  -> daemon broadcasts a WorkspaceUpdated snapshot or pane event
  -> ade-app reconciles its UI models with the snapshot
```

The UI reuses existing terminal parser objects by pane ID when applying a new snapshot. This keeps
the visible terminal state and selection intact across metadata, focus, layout, and workspace
updates.

## 16. Current boundaries and limitations

The following are important distinctions between current behavior and future or out-of-scope
capabilities:

- The app and ConPTY layer are Windows-only; the supported release target is Windows 11 x64.
- `vt100` is the current terminal state engine. The architecture notes identify `libghostty-vt` as
  a future replacement, but it is not integrated.
- Pane creation commands retain right/down names, but pane-count changes currently rebuild the
  canonical managed grid rather than preserving the requested split direction.
- Layouts are limited to six panes and at most two managed rows.
- Closing a workspace has no confirmation or undo.
- The app has no settings screen, tabs within a pane, shell picker, profile editor, search within
  terminal history, or session export feature in the current code.
- Automatic working-directory tracking and visual command blocks depend on the injected
  PowerShell prompt hook; the cmd.exe fallback does not provide them automatically.
- Git line totals come from the tracked working-tree and index diff against `HEAD`; untracked and
  binary-file line totals are not represented.
- The daemon preserves live sessions only while it remains running. Reboot restoration launches
  fresh shells rather than resuming arbitrary processes.
- Raw replay is bounded to 8 MiB per pane, while parsed UI scrollback is bounded to 10,000 lines.
- Live terminal broadcast is designed around one active attached UI subscriber, although multiple
  clients can connect for request/response operations such as snapshots.

For implementation-level process boundaries and planned terminal-engine evolution, see
[architecture.md](architecture.md).
