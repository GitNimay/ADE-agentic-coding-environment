# Architecture

## Current Slice

The repository currently establishes two Phase 1 boundaries:

- `ade-core` owns platform-independent, serializable workspace and split-layout state.
- `ade-pty` owns Windows ConPTY creation, process startup, pipe I/O, resizing, waiting, and handle
  cleanup.
- `ade-app` is the functional winit/wgpu desktop UI. It owns workspace presentation, terminal
  parsers, split interaction, and worker channels in the current single-process slice.

The split layout is a recursive binary tree. A right split creates a `columns` node and a down
split creates a `rows` node. Closing a pane collapses its parent into the remaining child. Ratios
are validated before they can enter persisted state.

Workspace terminal creation is capped at six by the daemon. Pane-count changes rebuild the binary
tree into a managed grid: one full pane, two or three columns, then two balanced rows for four to
six panes. The UI can resize persisted split ratios between pane-count changes, while the daemon
remains the authority for the cap and canonical arrangement.

## Target Process Boundary

Phase 1 uses two processes:

```text
ade-app.exe <-> per-user named pipe <-> ade-daemon.exe <-> one ConPTY per pane
```

The daemon owns ConPTY handles so sessions continue running when the UI is closed. It retains a
bounded raw VT replay buffer for every pane. On attach, the UI receives the authoritative SQLite
snapshot and replays buffered output into its terminal parsers before consuming live output. A
machine restart recreates persisted layouts with fresh shells; it cannot restore arbitrary process
memory.

## ConPTY Rules

The implementation follows these constraints from the Windows pseudoconsole API:

- Each pane gets an independent pseudoconsole and process tree.
- ConPTY pipes are synchronous.
- Production sessions must service output and input independently to prevent pipe backpressure
  from deadlocking the session.
- Child standard handles are marked invalid while the pseudoconsole process attribute is active.
  This prevents output inherited from the parent process from bypassing ConPTY.
- The two pipe handles supplied to `CreatePseudoConsole` are released after process creation. The
  host retains only its input writer and output reader.
- The pseudoconsole is closed before its host-side handles are discarded.

`ade-pty` exposes synchronous pipe halves so their Win32 behavior can be tested directly.
`ade-app` moves each input writer and output reader onto dedicated workers while retaining the
pseudoconsole and process handles in the pane session. The future daemon will take ownership of
those workers without changing the UI-facing terminal model.

## Desktop UI

The current application uses `eframe` with its wgpu renderer. `eframe` supplies the winit event
loop, accessibility bridge, clipboard integration, and immediate UI primitives. Terminal output
is parsed with `vt100` as an interim state engine and rendered cell-by-cell with ANSI, indexed, and
RGB colors. This parser will be replaced behind the terminal engine boundary when libghostty is
available.

## Terminal Engine Boundary

`libghostty-vt` remains the selected terminal state engine, but it is not integrated yet because
the local machine does not currently have Zig installed. Its unstable C API must remain behind an
internal Rust trait so no Ghostty types cross into workspace, IPC, or renderer crates.

The intended engine operations are:

- Feed output bytes
- Resize the terminal grid
- Produce a full grid and scrollback snapshot
- Produce incremental damage
- Encode keyboard and mouse input according to active terminal modes
- Validate bracketed paste

## Remaining Evolution

The current `vt100` state engine is an interim Rust implementation. A future engine swap should
install Zig and pin `libghostty-vt` behind the documented terminal-engine boundary without changing
workspace, storage, daemon, or rendering APIs.
