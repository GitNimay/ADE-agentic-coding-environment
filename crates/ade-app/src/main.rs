#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ade_core::{
    LayoutNode, MAX_TERMINALS_PER_WORKSPACE, PaneId, SessionStatus, SplitAxis, SplitDirection,
    Workspace,
};
use ade_protocol::{
    AppSnapshot, ClientRequest, PROTOCOL_VERSION, PaneSnapshot, ServerEvent, Versioned,
    WorkspaceSnapshot, pipe_name, read_frame, write_frame,
};
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{
    self, Color32, FontFamily, FontId, Key, KeyboardShortcut, Modifiers, RichText, Sense, Stroke,
    TextFormat, Vec2, text::LayoutJob,
};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Pipes::PeekNamedPipe;

const SCROLLBACK_LINES: usize = 10_000;
const TERMINAL_FONT_SIZE: f32 = 14.0;
const DIVIDER_SIZE: f32 = 6.0;
const MIN_PANE_WIDTH: f32 = 220.0;
const MIN_PANE_HEIGHT: f32 = 120.0;
const TERMINAL_SIDE_PADDING: f32 = 10.0;
const TERMINAL_BOTTOM_PADDING: f32 = 10.0;
const TERMINAL_FOOTER_HEIGHT: f32 = 28.0;
const GIT_REFRESH_INTERVAL: Duration = Duration::from_millis(1500);
const TERMINAL_DIVIDER_MARKER: &str = "__ADE_BLOCK_DIVIDER__";
const TERMINAL_DIVIDER_OFFSET: f32 = 7.0;
const TERMINAL_REVEAL_DURATION: Duration = Duration::from_millis(160);
const TERMINAL_REVEAL_OFFSET: f32 = 4.0;
const TERMINAL_CLOSE_DURATION: Duration = Duration::from_millis(220);
const SIDEBAR_BREAKPOINT: f32 = 600.0;
const SIDEBAR_WIDTH: f32 = 256.0;
const TABLET_SIDEBAR_WIDTH: f32 = 224.0;
const SIDEBAR_ROW_HEIGHT: f32 = 40.0;
const COLLAPSED_SIDEBAR_WIDTH: f32 = 56.0;
const WINDOW_TITLE_BAR_HEIGHT: f32 = 36.0;
const SIDEBAR_TRIGGER_WIDTH: f32 = 16.0;
const SIDEBAR_OPEN_DELAY: Duration = Duration::from_millis(180);
const SIDEBAR_CLOSE_DELAY: Duration = Duration::from_millis(450);

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().any(|argument| argument == "--daemon") {
        ade_daemon::run_daemon()?;
        return Ok(());
    }

    let app_icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/app-icon.png"))?;
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("")
            .with_icon(app_icon)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 360.0])
            .with_decorations(false)
            .with_maximized(true),
        ..Default::default()
    };

    eframe::run_native(
        "ADE",
        options,
        Box::new(|creation_context| Ok(Box::new(AdeApp::new(creation_context)))),
    )?;
    Ok(())
}

struct DaemonClient {
    requests: Sender<ClientRequest>,
    events: Receiver<ServerEvent>,
}

impl DaemonClient {
    fn connect(context: &egui::Context) -> io::Result<Self> {
        let pipe = connect_or_start_daemon()?;
        diagnostic_log("connected to daemon");
        let (request_tx, request_rx) = unbounded::<ClientRequest>();
        let (event_tx, event_rx) = unbounded::<ServerEvent>();

        let repaint_context = context.clone();
        thread::Builder::new()
            .name("ade-daemon-io".to_owned())
            .spawn(move || daemon_io_loop(pipe, &request_rx, &event_tx, &repaint_context))?;

        Ok(Self {
            requests: request_tx,
            events: event_rx,
        })
    }

    fn send(
        &self,
        request: ClientRequest,
    ) -> Result<(), crossbeam_channel::SendError<ClientRequest>> {
        diagnostic_log(&format!("queue request: {request:?}"));
        self.requests.send(request)
    }
}

fn connect_or_start_daemon() -> io::Result<File> {
    if let Ok(pipe) = open_daemon_pipe() {
        return Ok(pipe);
    }

    let executable = std::env::current_exe()?;
    Command::new(executable)
        .arg("--daemon")
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()?;

    let mut last_error = None;
    for _ in 0..60 {
        match open_daemon_pipe() {
            Ok(pipe) => return Ok(pipe),
            Err(error) => last_error = Some(error),
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(last_error
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "daemon startup timed out")))
}

fn open_daemon_pipe() -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(pipe_name())
}

fn daemon_io_loop(
    mut pipe: File,
    requests: &Receiver<ClientRequest>,
    events: &Sender<ServerEvent>,
    context: &egui::Context,
) {
    loop {
        for request in requests.try_iter() {
            diagnostic_log(&format!("write request: {request:?}"));
            if write_frame(&mut pipe, &Versioned::new(request)).is_err() {
                diagnostic_log("request writer disconnected");
                return;
            }
        }
        match pipe_bytes_available(&pipe) {
            Ok(0) => {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Ok(_) => {}
            Err(_) => {
                diagnostic_log("event reader disconnected");
                return;
            }
        }
        let Ok(event) = read_frame::<_, Versioned<ServerEvent>>(&mut pipe) else {
            diagnostic_log("event reader disconnected");
            return;
        };
        diagnostic_log(&format!("read event: {}", event_summary(&event.message)));
        if event.protocol_version != PROTOCOL_VERSION || events.send(event.message).is_err() {
            return;
        }
        context.request_repaint();
    }
}

fn pipe_bytes_available(pipe: &File) -> io::Result<u32> {
    let mut available = 0;
    let handle = HANDLE(pipe.as_raw_handle());
    // SAFETY: handle is a live named-pipe handle and available points to writable storage.
    match unsafe { PeekNamedPipe(handle, None, 0, None, Some(&raw mut available), None) } {
        Ok(()) => Ok(available),
        Err(_) => Err(io::Error::last_os_error()),
    }
}

fn event_summary(event: &ServerEvent) -> String {
    match event {
        ServerEvent::Attached { snapshot } => format!(
            "attached ({} workspaces, {} panes)",
            snapshot.workspaces.len(),
            snapshot.panes.len()
        ),
        ServerEvent::TerminalOutput { pane_id, data } => {
            format!("terminal output for {pane_id} ({} bytes)", data.len())
        }
        ServerEvent::WorkspaceUpdated { snapshot } => format!(
            "workspace update ({} workspaces, {} panes)",
            snapshot.workspaces.len(),
            snapshot.panes.len()
        ),
        ServerEvent::PaneStatus { pane_id, status } => {
            format!("pane status for {pane_id}: {status:?}")
        }
        ServerEvent::Error { message } => format!("error: {message}"),
    }
}

fn diagnostic_log(message: &str) {
    let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let directory = PathBuf::from(local_app_data).join("ADE");
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("ade-ui.log"))
    {
        let _ = writeln!(file, "{message}");
    }
}

struct AdeApp {
    workspaces: Vec<WorkspaceState>,
    active_workspace: usize,
    next_workspace_number: usize,
    error_message: Option<String>,
    palette_open: bool,
    palette_query: String,
    palette_selection: usize,
    client: Option<DaemonClient>,
    rename_workspace: Option<(ade_core::WorkspaceId, String)>,
    sidebar_open: bool,
    sidebar_hover_started: Option<Instant>,
    sidebar_left_at: Option<Instant>,
    terminal_limit_popup: bool,
}

impl AdeApp {
    fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        configure_style(&creation_context.egui_ctx);
        let client = DaemonClient::connect(&creation_context.egui_ctx);
        let error_message = client.as_ref().err().map(ToString::to_string);
        let mut app = Self {
            workspaces: Vec::new(),
            active_workspace: 0,
            next_workspace_number: 1,
            error_message,
            palette_open: false,
            palette_query: String::new(),
            palette_selection: 0,
            client: client.ok(),
            rename_workspace: None,
            sidebar_open: false,
            sidebar_hover_started: None,
            sidebar_left_at: None,
            terminal_limit_popup: false,
        };
        app.send(ClientRequest::Attach);
        app
    }

    fn send(&mut self, request: ClientRequest) {
        let Some(client) = &self.client else {
            self.error_message = Some("The ADE background daemon is not connected".to_owned());
            return;
        };
        if client.send(request).is_err() {
            self.error_message = Some("The ADE background daemon disconnected".to_owned());
        }
    }

    fn create_workspace(&mut self, name: String, root: &Path, _context: &egui::Context) {
        self.send(ClientRequest::CreateWorkspace {
            name,
            root: root.to_path_buf(),
        });
    }

    fn split_active(&mut self, direction: SplitDirection, context: &egui::Context) {
        let Some(workspace) = self.workspaces.get(self.active_workspace) else {
            return;
        };
        if terminal_limit_reached(workspace.model.layout.pane_ids().len()) {
            self.terminal_limit_popup = true;
            return;
        }
        let request = workspace.model.active_pane_id.map_or(
            ClientRequest::CreatePane {
                workspace_id: workspace.model.id,
            },
            |target| ClientRequest::SplitPane {
                workspace_id: workspace.model.id,
                target,
                direction,
            },
        );
        self.send(request);
        context.request_repaint();
    }

    fn close_active_pane(&mut self, context: &egui::Context) {
        let Some(workspace) = self.workspaces.get_mut(self.active_workspace) else {
            return;
        };
        if let Some(pane_id) = workspace.model.active_pane_id
            && let Some(pane) = workspace.panes.get_mut(&pane_id)
            && pane.close_started_at.is_none()
        {
            pane.close_started_at = Some(Instant::now());
            context.request_repaint();
        }
    }

    fn finish_pane_closes(&mut self, context: &egui::Context) {
        let mut panes_to_close = Vec::new();
        for workspace in &mut self.workspaces {
            for pane in workspace.panes.values_mut() {
                let Some(started_at) = pane.close_started_at else {
                    continue;
                };
                if !pane.close_request_sent {
                    if started_at.elapsed() >= TERMINAL_CLOSE_DURATION {
                        pane.close_request_sent = true;
                        panes_to_close.push(pane.id);
                    } else {
                        context.request_repaint();
                    }
                }
            }
        }
        for pane_id in panes_to_close {
            self.send(ClientRequest::ClosePane { pane_id });
        }
    }

    fn drain_daemon_events(&mut self, context: &egui::Context) {
        let events: Vec<_> = self
            .client
            .as_ref()
            .map(|client| client.events.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                ServerEvent::Attached { snapshot } => {
                    let create_initial = snapshot.workspaces.is_empty();
                    self.apply_snapshot(snapshot);
                    if create_initial {
                        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                        self.create_workspace("Workspace 1".to_owned(), &root, context);
                    }
                }
                ServerEvent::WorkspaceUpdated { snapshot } => self.apply_snapshot(snapshot),
                ServerEvent::TerminalOutput { pane_id, data } => {
                    if let Some(pane) = self.pane_mut(pane_id) {
                        pane.process_output(&data);
                    }
                }
                ServerEvent::PaneStatus { pane_id, status } => {
                    if let Some(pane) = self.pane_mut(pane_id) {
                        pane.status = status;
                    }
                }
                ServerEvent::Error { message } => self.error_message = Some(message),
            }
        }
    }

    fn pane_mut(&mut self, pane_id: PaneId) -> Option<&mut TerminalPane> {
        self.workspaces
            .iter_mut()
            .find_map(|workspace| workspace.panes.get_mut(&pane_id))
    }

    fn apply_snapshot(&mut self, snapshot: AppSnapshot) {
        let mut existing: HashMap<PaneId, TerminalPane> = self
            .workspaces
            .drain(..)
            .flat_map(|workspace| workspace.panes)
            .collect();
        let pane_metadata: HashMap<PaneId, PaneSnapshot> = snapshot
            .panes
            .into_iter()
            .map(|pane| (pane.id, pane))
            .collect();

        self.workspaces = snapshot
            .workspaces
            .into_iter()
            .map(|workspace| {
                let panes = workspace
                    .layout
                    .pane_ids()
                    .into_iter()
                    .filter_map(|pane_id| {
                        let metadata = pane_metadata.get(&pane_id)?;
                        let pane = existing.remove(&pane_id).map_or_else(
                            || TerminalPane::new(metadata),
                            |mut pane| {
                                pane.update_metadata(metadata);
                                pane
                            },
                        );
                        Some((pane_id, pane))
                    })
                    .collect();
                WorkspaceState::from_snapshot(workspace, panes)
            })
            .collect();

        self.active_workspace = snapshot
            .active_workspace_id
            .and_then(|active| {
                self.workspaces
                    .iter()
                    .position(|workspace| workspace.model.id == active)
            })
            .unwrap_or(0)
            .min(self.workspaces.len().saturating_sub(1));
        self.next_workspace_number = self.workspaces.len() + 1;
    }

    fn handle_shortcuts(&mut self, context: &egui::Context) {
        if context.input_mut(|input| {
            input.consume_shortcut(&shortcut(Key::P))
                || input.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::K))
        }) {
            self.palette_open = true;
            self.palette_query.clear();
            self.palette_selection = 0;
        }
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::N))) {
            self.request_new_workspace(context);
        }
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::D))) {
            self.split_active(SplitDirection::Right, context);
        }
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::E))) {
            self.split_active(SplitDirection::Down, context);
        }
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::W))) {
            self.close_active_pane(context);
        }
        if context.input_mut(|input| input.consume_key(Modifiers::NONE, Key::F2))
            && let Some(workspace) = self.workspaces.get(self.active_workspace)
        {
            self.rename_workspace = Some((workspace.model.id, workspace.model.name.clone()));
        }
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::C)))
            && let Some(workspace) = self.workspaces.get(self.active_workspace)
            && let Some(pane_id) = workspace.model.active_pane_id
            && let Some(pane) = workspace.panes.get(&pane_id)
            && let Some(text) = pane.selected_text()
        {
            context.copy_text(text);
        }
        if context.input_mut(|input| input.consume_shortcut(&shortcut(Key::V))) {
            match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                Ok(text) => {
                    if let Some(workspace) = self.workspaces.get(self.active_workspace) {
                        let paste = workspace.model.active_pane_id.and_then(|pane_id| {
                            workspace
                                .panes
                                .get(&pane_id)
                                .map(|pane| (pane_id, pane.paste_bytes(&text)))
                        });
                        if let Some((pane_id, data)) = paste {
                            self.send(ClientRequest::Input { pane_id, data });
                        }
                    }
                }
                Err(error) => self.error_message = Some(format!("Clipboard paste failed: {error}")),
            }
        }
        let previous_pane = context.input_mut(|input| {
            input.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::CTRL | Modifiers::ALT,
                Key::ArrowLeft,
            )) || input.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::CTRL | Modifiers::ALT,
                Key::ArrowUp,
            ))
        });
        let next_pane = context.input_mut(|input| {
            input.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::CTRL | Modifiers::ALT,
                Key::ArrowRight,
            )) || input.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::CTRL | Modifiers::ALT,
                Key::ArrowDown,
            ))
        });
        if previous_pane || next_pane {
            self.move_pane_focus(next_pane);
        }
        if context.input_mut(|input| {
            input.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::PageDown))
        }) && !self.workspaces.is_empty()
        {
            self.focus_relative_workspace(true);
        }
        if context.input_mut(|input| {
            input.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::PageUp))
        }) && !self.workspaces.is_empty()
        {
            self.focus_relative_workspace(false);
        }
    }

    fn focus_relative_workspace(&mut self, forward: bool) {
        if self.workspaces.is_empty() {
            return;
        }
        self.active_workspace = if forward {
            (self.active_workspace + 1) % self.workspaces.len()
        } else {
            self.active_workspace
                .checked_sub(1)
                .unwrap_or(self.workspaces.len() - 1)
        };
        let workspace_id = self.workspaces[self.active_workspace].model.id;
        self.send(ClientRequest::FocusWorkspace { workspace_id });
    }

    fn request_new_workspace(&mut self, context: &egui::Context) {
        let initial = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if let Some(root) = rfd::FileDialog::new().set_directory(initial).pick_folder() {
            let name = root.file_name().and_then(|name| name.to_str()).map_or_else(
                || format!("Workspace {}", self.next_workspace_number),
                str::to_owned,
            );
            self.next_workspace_number += 1;
            self.create_workspace(name, &root, context);
        }
    }

    fn move_pane_focus(&mut self, forward: bool) {
        let Some(workspace) = self.workspaces.get_mut(self.active_workspace) else {
            return;
        };
        let panes = workspace.model.layout.pane_ids();
        if panes.is_empty() {
            return;
        }
        let Some(index) = panes
            .iter()
            .position(|pane| Some(*pane) == workspace.model.active_pane_id)
        else {
            return;
        };
        let next = if forward {
            (index + 1) % panes.len()
        } else {
            index.checked_sub(1).unwrap_or(panes.len() - 1)
        };
        let pane_id = panes[next];
        workspace.model.active_pane_id = Some(pane_id);
        self.send(ClientRequest::FocusPane { pane_id });
    }

    #[allow(clippy::too_many_lines)]
    fn sidebar(&mut self, root_ui: &mut egui::Ui, context: &egui::Context) {
        if root_ui.available_width() <= SIDEBAR_BREAKPOINT {
            self.compact_workspace_bar(root_ui, context);
            return;
        }

        let mut action = None;
        let mut create_workspace = false;
        let tablet = root_ui.available_width() <= 960.0;
        let expanded_width = if tablet {
            TABLET_SIDEBAR_WIDTH
        } else {
            SIDEBAR_WIDTH
        };
        let expansion = context.animate_bool_with_time_and_easing(
            egui::Id::new("workspace-sidebar-animation"),
            self.sidebar_open,
            0.16,
            egui::emath::easing::cubic_out,
        );
        let sidebar_width = egui::lerp(COLLAPSED_SIDEBAR_WIDTH..=expanded_width, expansion);
        let mut context_menu_open = false;
        let panel = egui::Panel::left("workspace-sidebar")
            .resizable(false)
            .exact_size(sidebar_width)
            .frame(
                egui::Frame::NONE
                    .fill(surface_primary())
                    .stroke(Stroke::new(1.0, border())),
            )
            .show(root_ui, |ui| {
                if sidebar_width < 144.0 {
                    compact_sidebar_rail(
                        ui,
                        &self.workspaces,
                        self.active_workspace,
                        &mut action,
                        &mut context_menu_open,
                        &mut create_workspace,
                    );
                    return;
                }
                let (header_rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), WINDOW_TITLE_BAR_HEIGHT),
                    Sense::hover(),
                );
                let mut header = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(header_rect.shrink2(Vec2::new(12.0, 2.0)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                header.label(
                    RichText::new("Workspaces")
                        .size(13.0)
                        .strong()
                        .color(text_secondary()),
                );
                header.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if compact_icon_button(ui, "+", "New workspace")
                        .on_hover_text("New workspace")
                        .clicked()
                    {
                        create_workspace = true;
                    }
                });
                egui::Frame::NONE
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 8,
                        top: 8,
                        bottom: 8,
                    })
                    .show(ui, |ui| {
                        for (index, workspace) in self.workspaces.iter().enumerate() {
                            if let Some(next) = workspace_row(
                                ui,
                                workspace,
                                index,
                                index == self.active_workspace,
                                &mut context_menu_open,
                            ) {
                                action = Some(next);
                            }
                        }
                    });
            });

        let pointer = context.input(|input| input.pointer.hover_pos());
        let edge_hovered = pointer.is_some_and(|position| {
            position.x <= panel.response.rect.left() + SIDEBAR_TRIGGER_WIDTH
                && panel.response.rect.y_range().contains(position.y)
        });
        let panel_hovered = pointer.is_some_and(|position| panel.response.rect.contains(position));
        self.update_sidebar_hover(panel_hovered || edge_hovered, context_menu_open);

        if create_workspace {
            self.request_new_workspace(context);
        }
        self.handle_workspace_action(action);
    }

    fn update_sidebar_hover(&mut self, hovered: bool, context_menu_open: bool) {
        let now = Instant::now();
        if context_menu_open {
            self.sidebar_open = true;
            self.sidebar_left_at = None;
            return;
        }

        if self.sidebar_open {
            self.sidebar_hover_started = None;
            if hovered {
                self.sidebar_left_at = None;
            } else {
                let left_at = self.sidebar_left_at.get_or_insert(now);
                if now.duration_since(*left_at) >= SIDEBAR_CLOSE_DELAY {
                    self.sidebar_open = false;
                    self.sidebar_left_at = None;
                }
            }
        } else {
            self.sidebar_left_at = None;
            if hovered {
                let hover_started = self.sidebar_hover_started.get_or_insert(now);
                if now.duration_since(*hover_started) >= SIDEBAR_OPEN_DELAY {
                    self.sidebar_open = true;
                    self.sidebar_hover_started = None;
                }
            } else {
                self.sidebar_hover_started = None;
            }
        }
    }

    fn compact_workspace_bar(&mut self, root_ui: &mut egui::Ui, context: &egui::Context) {
        let mut action = None;
        let mut create_workspace = false;
        egui::Panel::top("compact-workspace-bar")
            .exact_size(40.0)
            .frame(
                egui::Frame::NONE
                    .fill(surface_primary())
                    .inner_margin(8.0)
                    .stroke(Stroke::new(1.0, border())),
            )
            .show(root_ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.menu_button(
                        RichText::new(self.workspaces.get(self.active_workspace).map_or_else(
                            || "Workspaces".to_owned(),
                            |workspace| compact_text(&workspace.model.name, 30),
                        ))
                        .size(14.0),
                        |ui| {
                            ui.set_min_width(
                                (context.content_rect().width() - 32.0).clamp(200.0, 280.0),
                            );
                            for (index, workspace) in self.workspaces.iter().enumerate() {
                                let response = menu_item(
                                    ui,
                                    &compact_text(&workspace.model.name, 30),
                                    text_primary(),
                                    index == self.active_workspace,
                                    36.0,
                                );
                                if response.clicked() {
                                    action = Some(WorkspaceAction::Focus(index));
                                    ui.close();
                                }
                                workspace_context_menu(&response, workspace, &mut action);
                            }
                            ui.separator();
                            if menu_item(ui, "New workspace", text_primary(), false, 36.0).clicked()
                            {
                                create_workspace = true;
                                ui.close();
                            }
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if compact_icon_button(ui, "+", "New workspace")
                            .on_hover_text("New workspace")
                            .clicked()
                        {
                            create_workspace = true;
                        }
                    });
                });
            });

        if create_workspace {
            self.request_new_workspace(context);
        }
        self.handle_workspace_action(action);
    }

    fn handle_workspace_action(&mut self, action: Option<WorkspaceAction>) {
        match action {
            Some(WorkspaceAction::Focus(index)) => {
                self.active_workspace = index;
                let workspace_id = self.workspaces[index].model.id;
                self.send(ClientRequest::FocusWorkspace { workspace_id });
            }
            Some(WorkspaceAction::Edit(workspace_id, name)) => {
                self.rename_workspace = Some((workspace_id, name));
            }
            Some(WorkspaceAction::Close(workspace_id)) => {
                self.send(ClientRequest::CloseWorkspace { workspace_id });
            }
            None => {}
        }
    }

    fn workspace_dialogs(&mut self, context: &egui::Context) {
        let mut rename_action = None;
        let mut cancel_rename = false;
        if let Some((workspace_id, name)) = &mut self.rename_workspace {
            egui::Window::new("Rename workspace")
                .id(egui::Id::new("rename-workspace"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(context, |ui| {
                    let input_width = (context.content_rect().width() - 80.0).clamp(240.0, 360.0);
                    let response = ui.add(
                        egui::TextEdit::singleline(name)
                            .desired_width(input_width)
                            .hint_text("Workspace name"),
                    );
                    response.request_focus();
                    ui.horizontal(|ui| {
                        if ui.button("Rename").clicked()
                            || (response.lost_focus()
                                && ui.input(|input| input.key_pressed(Key::Enter)))
                        {
                            rename_action = Some((*workspace_id, name.trim().to_owned()));
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_rename = true;
                        }
                    });
                });
        }
        if let Some((workspace_id, name)) = rename_action {
            if !name.is_empty() {
                self.send(ClientRequest::RenameWorkspace { workspace_id, name });
            }
            self.rename_workspace = None;
        } else if cancel_rename || context.input(|input| input.key_pressed(Key::Escape)) {
            self.rename_workspace = None;
        }
    }

    #[allow(clippy::too_many_lines)]
    fn command_palette(&mut self, context: &egui::Context) {
        if self.palette_open
            && context.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.palette_open = false;
        }
        let reveal = context.animate_bool_with_time_and_easing(
            egui::Id::new("command-palette-reveal"),
            self.palette_open,
            0.2,
            egui::emath::easing::cubic_out,
        );
        if !self.palette_open && reveal <= 0.001 {
            return;
        }

        let filtered: Vec<_> = PALETTE_COMMANDS
            .iter()
            .filter(|entry| palette_matches(entry.label, &self.palette_query))
            .collect();
        self.palette_selection = self.palette_selection.min(filtered.len().saturating_sub(1));

        let move_up = context.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp));
        let move_down =
            context.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
        if !filtered.is_empty() {
            if move_up {
                self.palette_selection = self
                    .palette_selection
                    .checked_sub(1)
                    .unwrap_or(filtered.len() - 1);
            } else if move_down {
                self.palette_selection = (self.palette_selection + 1) % filtered.len();
            }
        }
        let activate = context.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
        let mut chosen = None;
        let content_rect = context.content_rect();
        let backdrop = egui::Area::new(egui::Id::new("command-palette-backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(content_rect.min)
            .show(context, |ui| {
                let (rect, response) = ui.allocate_exact_size(content_rect.size(), Sense::click());
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    Color32::from_black_alpha(150).gamma_multiply(reveal),
                );
                response
            })
            .inner;
        if backdrop.clicked() {
            self.palette_open = false;
        }
        let final_width = (content_rect.width() - 32.0).clamp(320.0, 440.0);
        let panel_width = egui::lerp(final_width * 0.965..=final_width, reveal);
        let panel_top = egui::lerp(58.0..=72.0, reveal);
        egui::Area::new(egui::Id::new("command-palette"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(
                content_rect.center().x,
                content_rect.top() + panel_top,
            ))
            .show(context, |ui| {
                ui.set_width(panel_width);
                egui::Frame::window(&context.style_of(egui::Theme::Dark))
                    .fill(Color32::BLACK)
                    .corner_radius(12.0)
                    .stroke(Stroke::new(1.0, Color32::from_rgb(46, 46, 46)))
                    .inner_margin(egui::Margin::same(0))
                    .show(ui, |ui| {
                        ui.set_width(panel_width - 2.0);
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    paint_palette_search_icon(ui);
                                    let response = ui.add(
                                        egui::TextEdit::singleline(&mut self.palette_query)
                                            .hint_text("Type a command or search…")
                                            .desired_width(f32::INFINITY)
                                            .font(FontId::proportional(14.0))
                                            .frame(egui::Frame::NONE),
                                    );
                                    response.request_focus();
                                    if response.changed() {
                                        self.palette_selection = 0;
                                    }
                                    palette_keycap(ui, "Esc");
                                });
                            });
                        ui.painter().hline(
                            ui.max_rect().x_range(),
                            ui.min_rect().bottom(),
                            Stroke::new(1.0, border()),
                        );

                        egui::ScrollArea::vertical()
                            .max_height(196.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.add_space(6.0);
                                if filtered.is_empty() {
                                    palette_empty_state(ui, &self.palette_query);
                                } else {
                                    let mut previous_group = None;
                                    for (index, entry) in filtered.iter().enumerate() {
                                        if previous_group != Some(entry.group) {
                                            palette_group_heading(ui, entry.group);
                                            previous_group = Some(entry.group);
                                        }
                                        let response = palette_command_row(
                                            ui,
                                            entry,
                                            index == self.palette_selection,
                                        );
                                        if response.hovered() {
                                            self.palette_selection = index;
                                        }
                                        if response.clicked() {
                                            chosen = Some(entry.command);
                                        }
                                    }
                                }
                                ui.add_space(6.0);
                            });

                        palette_footer(ui, filtered.len());
                    });
            });

        if activate && !filtered.is_empty() {
            chosen = Some(filtered[self.palette_selection].command);
        }
        if let Some(command) = chosen {
            self.palette_open = false;
            self.execute_palette_command(command, context);
        }
    }

    fn execute_palette_command(&mut self, command: PaletteCommand, context: &egui::Context) {
        match command {
            PaletteCommand::NewWorkspace => self.request_new_workspace(context),
            PaletteCommand::SplitRight => self.split_active(SplitDirection::Right, context),
            PaletteCommand::SplitDown => self.split_active(SplitDirection::Down, context),
            PaletteCommand::ClosePane => self.close_active_pane(context),
            PaletteCommand::RenameWorkspace => {
                if let Some(workspace) = self.workspaces.get(self.active_workspace) {
                    self.rename_workspace =
                        Some((workspace.model.id, workspace.model.name.clone()));
                }
            }
            PaletteCommand::CloseWorkspace => {
                if let Some(workspace) = self.workspaces.get(self.active_workspace) {
                    self.send(ClientRequest::CloseWorkspace {
                        workspace_id: workspace.model.id,
                    });
                }
            }
            PaletteCommand::NextWorkspace => self.focus_relative_workspace(true),
            PaletteCommand::PreviousWorkspace => self.focus_relative_workspace(false),
        }
    }

    fn show_terminal_limit_popup(&mut self, context: &egui::Context) {
        if !self.terminal_limit_popup {
            return;
        }
        let modal_width = (context.content_rect().width() - 32.0).clamp(304.0, 400.0);
        let modal_frame = egui::Frame::NONE
            .fill(Color32::from_rgb(10, 10, 10))
            .stroke(Stroke::new(1.0, border()))
            .corner_radius(12.0)
            .shadow(egui::epaint::Shadow {
                offset: [0, 16],
                blur: 40,
                spread: 0,
                color: Color32::from_black_alpha(210),
            });
        let response = egui::Modal::new(egui::Id::new("terminal-limit-popup"))
            .backdrop_color(Color32::from_black_alpha(176))
            .frame(modal_frame)
            .show(context, |ui| {
                ui.set_width(modal_width);
                egui::Frame::NONE
                    .inner_margin(egui::Margin {
                        left: 24,
                        right: 24,
                        top: 22,
                        bottom: 20,
                    })
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Terminal Limit Reached")
                                .font(FontId::proportional(16.0))
                                .strong()
                                .color(text_primary()),
                        );
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(format!(
                                "This workspace is limited to {MAX_TERMINALS_PER_WORKSPACE} terminals. Close one before opening a new terminal."
                            ))
                            .font(FontId::proportional(14.0))
                            .color(text_secondary()),
                        );
                    });

                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.min_rect().bottom(),
                    Stroke::new(1.0, border()),
                );
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let done = modal_primary_button(ui, "Done");
                            done.request_focus();
                            if done.clicked()
                                || ui.input(|input| input.key_pressed(Key::Enter))
                            {
                                ui.close();
                            }
                        });
                    });
            });
        if response.should_close() {
            self.terminal_limit_popup = false;
        }
    }
}

impl eframe::App for AdeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        self.drain_daemon_events(&context);
        self.handle_shortcuts(&context);
        self.finish_pane_closes(&context);
        let compact = ui.available_width() <= SIDEBAR_BREAKPOINT;
        if compact {
            window_title_bar(ui, &context);
            self.sidebar(ui, &context);
        } else {
            self.sidebar(ui, &context);
            window_title_bar(ui, &context);
        }

        let requests = self.client.as_ref().map(|client| client.requests.clone());
        let terminal_input_enabled = !self.palette_open && self.rename_workspace.is_none();
        let mut updated_layout = None;
        let mut create_terminal = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(terminal_background()))
            .show(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                if let Some(workspace) = self.workspaces.get_mut(self.active_workspace)
                    && let Some(requests) = &requests
                {
                    if matches!(workspace.model.layout, LayoutNode::Empty) {
                        let response = ui.interact(
                            rect,
                            egui::Id::new(("empty-workspace", workspace.model.id)),
                            Sense::click(),
                        );
                        ui.painter().text(
                            rect.center() - Vec2::new(0.0, 10.0),
                            egui::Align2::CENTER_CENTER,
                            "No terminals",
                            FontId::proportional(18.0),
                            text_primary(),
                        );
                        ui.painter().text(
                            rect.center() + Vec2::new(0.0, 16.0),
                            egui::Align2::CENTER_CENTER,
                            "Click to open a terminal",
                            FontId::proportional(13.0),
                            text_secondary(),
                        );
                        if response.clicked() {
                            create_terminal = Some(workspace.model.id);
                        }
                    } else {
                        let changed = render_layout(
                            ui,
                            rect,
                            &mut workspace.model.layout,
                            &mut workspace.panes,
                            &mut workspace.model.active_pane_id,
                            requests,
                            terminal_input_enabled,
                            "root",
                        );
                        if changed {
                            updated_layout =
                                Some((workspace.model.id, workspace.model.layout.clone()));
                        }
                    }
                }
            });
        if let Some(workspace_id) = create_terminal {
            self.send(ClientRequest::CreatePane { workspace_id });
        }
        if let Some((workspace_id, layout)) = updated_layout {
            self.send(ClientRequest::UpdateLayout {
                workspace_id,
                layout,
            });
        }

        if let Some(message) = self.error_message.clone() {
            egui::Window::new("ADE error")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(&context, |ui| {
                    ui.label(message);
                    if ui.button("Dismiss").clicked() {
                        self.error_message = None;
                    }
                });
        }
        self.show_terminal_limit_popup(&context);
        self.workspace_dialogs(&context);
        self.command_palette(&context);
        context.request_repaint_after(Duration::from_millis(33));
    }
}

#[derive(Clone, Copy)]
enum WindowControl {
    Minimize,
    Maximize,
    Close,
}

fn window_title_bar(root_ui: &mut egui::Ui, context: &egui::Context) {
    let maximized = context.input(|input| input.viewport().maximized.unwrap_or(false));
    let panel = egui::Panel::top("window-title-bar")
        .exact_size(WINDOW_TITLE_BAR_HEIGHT)
        .frame(egui::Frame::NONE.fill(surface_primary()))
        .show(root_ui, |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if window_control_button(ui, WindowControl::Close, maximized).clicked() {
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if window_control_button(ui, WindowControl::Maximize, maximized).clicked() {
                    context.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
                if window_control_button(ui, WindowControl::Minimize, maximized).clicked() {
                    context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }

                let drag_rect = ui.available_rect_before_wrap();
                let drag = ui.interact(
                    drag_rect,
                    egui::Id::new("window-title-drag-region"),
                    Sense::click_and_drag(),
                );
                if drag.double_clicked() {
                    context.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                } else if drag.drag_started() {
                    context.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
            });
        });
    let app_rect = context.content_rect();
    let divider_height = 1.0 / context.pixels_per_point();
    let painter = context.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("window-title-bar-borders"),
    ));
    painter.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(app_rect.left(), panel.response.rect.top()),
            Vec2::new(app_rect.width(), divider_height),
        ),
        0.0,
        border(),
    );
    painter.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(
                app_rect.left(),
                panel.response.rect.bottom() - divider_height,
            ),
            Vec2::new(app_rect.width(), divider_height),
        ),
        0.0,
        border(),
    );
}

fn window_control_button(
    ui: &mut egui::Ui,
    control: WindowControl,
    maximized: bool,
) -> egui::Response {
    let label = match control {
        WindowControl::Minimize => "Minimize",
        WindowControl::Maximize if maximized => "Restore",
        WindowControl::Maximize => "Maximize",
        WindowControl::Close => "Close",
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(46.0, 36.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            0.0,
            if matches!(control, WindowControl::Close) {
                Color32::from_rgb(196, 43, 28)
            } else {
                surface_hover()
            },
        );
    }

    let center = rect.center();
    let stroke = Stroke::new(1.0, text_primary());
    match control {
        WindowControl::Minimize => {
            ui.painter().line_segment(
                [center + Vec2::new(-5.0, 3.0), center + Vec2::new(5.0, 3.0)],
                stroke,
            );
        }
        WindowControl::Maximize if maximized => {
            let back =
                egui::Rect::from_center_size(center + Vec2::new(1.5, -1.5), Vec2::new(8.0, 7.0));
            let front =
                egui::Rect::from_center_size(center + Vec2::new(-1.5, 1.5), Vec2::new(8.0, 7.0));
            ui.painter()
                .rect_stroke(back, 0.0, stroke, egui::StrokeKind::Inside);
            ui.painter().rect_filled(front, 0.0, surface_primary());
            ui.painter()
                .rect_stroke(front, 0.0, stroke, egui::StrokeKind::Inside);
        }
        WindowControl::Maximize => {
            ui.painter().rect_stroke(
                egui::Rect::from_center_size(center, Vec2::new(10.0, 9.0)),
                0.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        WindowControl::Close => {
            ui.painter().line_segment(
                [center - Vec2::splat(4.0), center + Vec2::splat(4.0)],
                stroke,
            );
            ui.painter().line_segment(
                [center + Vec2::new(-4.0, 4.0), center + Vec2::new(4.0, -4.0)],
                stroke,
            );
        }
    }
    response
}

#[derive(Clone, Copy)]
enum PaletteCommand {
    NewWorkspace,
    SplitRight,
    SplitDown,
    ClosePane,
    RenameWorkspace,
    CloseWorkspace,
    NextWorkspace,
    PreviousWorkspace,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum PaletteGroup {
    Actions,
    Navigation,
}

impl PaletteGroup {
    const fn label(self) -> &'static str {
        match self {
            Self::Actions => "Actions",
            Self::Navigation => "Navigation",
        }
    }
}

struct PaletteEntry {
    label: &'static str,
    command: PaletteCommand,
    shortcut: &'static str,
    icon: &'static str,
    group: PaletteGroup,
}

const PALETTE_COMMANDS: [PaletteEntry; 8] = [
    PaletteEntry {
        label: "New Workspace",
        command: PaletteCommand::NewWorkspace,
        shortcut: "Ctrl Shift N",
        icon: "+",
        group: PaletteGroup::Actions,
    },
    PaletteEntry {
        label: "Split Pane Right",
        command: PaletteCommand::SplitRight,
        shortcut: "Ctrl Shift D",
        icon: "→",
        group: PaletteGroup::Actions,
    },
    PaletteEntry {
        label: "Split Pane Down",
        command: PaletteCommand::SplitDown,
        shortcut: "Ctrl Shift E",
        icon: "↓",
        group: PaletteGroup::Actions,
    },
    PaletteEntry {
        label: "Close Active Pane",
        command: PaletteCommand::ClosePane,
        shortcut: "Ctrl Shift W",
        icon: "×",
        group: PaletteGroup::Actions,
    },
    PaletteEntry {
        label: "Rename Workspace…",
        command: PaletteCommand::RenameWorkspace,
        shortcut: "F2",
        icon: "A",
        group: PaletteGroup::Actions,
    },
    PaletteEntry {
        label: "Close Workspace",
        command: PaletteCommand::CloseWorkspace,
        shortcut: "",
        icon: "−",
        group: PaletteGroup::Actions,
    },
    PaletteEntry {
        label: "Next Workspace",
        command: PaletteCommand::NextWorkspace,
        shortcut: "Ctrl PgDn",
        icon: "›",
        group: PaletteGroup::Navigation,
    },
    PaletteEntry {
        label: "Previous Workspace",
        command: PaletteCommand::PreviousWorkspace,
        shortcut: "Ctrl PgUp",
        icon: "‹",
        group: PaletteGroup::Navigation,
    },
];

fn palette_matches(label: &str, query: &str) -> bool {
    let query = query.trim();
    query.is_empty()
        || label
            .to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
}

fn paint_palette_search_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
    let center = rect.center() - Vec2::new(1.0, 1.0);
    let stroke = Stroke::new(1.4, text_secondary());
    ui.painter().circle_stroke(center, 5.0, stroke);
    ui.painter().line_segment(
        [center + Vec2::new(3.7, 3.7), center + Vec2::new(7.0, 7.0)],
        stroke,
    );
}

fn palette_keycap(ui: &mut egui::Ui, label: &str) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(26, 26, 26))
        .stroke(Stroke::new(1.0, Color32::from_rgb(46, 46, 46)))
        .corner_radius(5.0)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(label)
                    .font(FontId::monospace(10.5))
                    .color(text_secondary()),
            );
        });
}

fn palette_group_heading(ui: &mut egui::Ui, group: PaletteGroup) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    ui.painter().text(
        egui::pos2(rect.left() + 14.0, rect.center().y + 1.0),
        egui::Align2::LEFT_CENTER,
        group.label(),
        FontId::proportional(11.0),
        text_disabled(),
    );
}

#[allow(clippy::cast_precision_loss)]
fn palette_command_row(ui: &mut egui::Ui, entry: &PaletteEntry, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 40.0), Sense::click());
    let row_rect = rect.shrink2(Vec2::new(7.0, 1.0));
    if selected || response.hovered() {
        ui.painter().rect_filled(
            row_rect,
            6.0,
            if response.is_pointer_button_down_on() {
                Color32::from_rgb(31, 31, 31)
            } else {
                Color32::from_rgb(26, 26, 26)
            },
        );
        ui.painter().rect_stroke(
            row_rect,
            6.0,
            Stroke::new(1.0, Color32::from_rgb(46, 46, 46)),
            egui::StrokeKind::Inside,
        );
    }

    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(row_rect.left() + 21.0, row_rect.center().y),
        Vec2::splat(26.0),
    );
    ui.painter().rect_filled(
        icon_rect,
        6.0,
        if selected {
            Color32::from_rgb(41, 41, 41)
        } else {
            Color32::from_rgb(26, 26, 26)
        },
    );
    ui.painter().rect_stroke(
        icon_rect,
        6.0,
        Stroke::new(1.0, Color32::from_rgb(46, 46, 46)),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        icon_rect.center(),
        egui::Align2::CENTER_CENTER,
        entry.icon,
        FontId::proportional(13.0),
        if selected {
            text_primary()
        } else {
            text_secondary()
        },
    );
    ui.painter().text(
        egui::pos2(icon_rect.right() + 11.0, row_rect.center().y),
        egui::Align2::LEFT_CENTER,
        entry.label,
        FontId::proportional(13.0),
        text_primary(),
    );

    if !entry.shortcut.is_empty() {
        let key_width = entry.shortcut.chars().count() as f32 * 6.4 + 14.0;
        let key_rect = egui::Rect::from_center_size(
            egui::pos2(
                row_rect.right() - key_width * 0.5 - 9.0,
                row_rect.center().y,
            ),
            Vec2::new(key_width, 24.0),
        );
        ui.painter()
            .rect_filled(key_rect, 5.0, Color32::from_rgb(26, 26, 26));
        ui.painter().rect_stroke(
            key_rect,
            5.0,
            Stroke::new(1.0, Color32::from_rgb(46, 46, 46)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            key_rect.center(),
            egui::Align2::CENTER_CENTER,
            entry.shortcut,
            FontId::monospace(10.5),
            text_secondary(),
        );
    }
    if selected {
        response.scroll_to_me(None);
    }
    response
}

fn palette_empty_state(ui: &mut egui::Ui, query: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 116.0), Sense::hover());
    ui.painter().text(
        rect.center() - Vec2::new(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        "No commands found",
        FontId::proportional(14.0),
        text_primary(),
    );
    ui.painter().text(
        rect.center() + Vec2::new(0.0, 13.0),
        egui::Align2::CENTER_CENTER,
        format!("Try a different search than “{}”", query.trim()),
        FontId::proportional(12.0),
        text_secondary(),
    );
}

fn palette_footer(ui: &mut egui::Ui, result_count: usize) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.top(), Stroke::new(1.0, border()));
    ui.painter().text(
        egui::pos2(rect.left() + 14.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        format!("{result_count} commands"),
        FontId::proportional(11.0),
        text_disabled(),
    );
    ui.painter().text(
        egui::pos2(rect.right() - 14.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        "↑↓  Navigate    ↵  Select    Esc  Close",
        FontId::monospace(10.5),
        text_secondary(),
    );
}

enum WorkspaceAction {
    Focus(usize),
    Edit(ade_core::WorkspaceId, String),
    Close(ade_core::WorkspaceId),
}

struct WorkspaceState {
    model: Workspace,
    panes: HashMap<PaneId, TerminalPane>,
}

impl WorkspaceState {
    fn from_snapshot(snapshot: WorkspaceSnapshot, panes: HashMap<PaneId, TerminalPane>) -> Self {
        Self {
            model: Workspace {
                id: snapshot.id,
                name: snapshot.name,
                root_directory: snapshot.root,
                layout: snapshot.layout,
                active_pane_id: snapshot.active_pane_id,
            },
            panes,
        }
    }

    fn any_running(&self) -> bool {
        self.panes.values().any(|pane| {
            matches!(
                pane.status,
                SessionStatus::Starting | SessionStatus::Running
            )
        })
    }
}

fn compact_sidebar_rail(
    ui: &mut egui::Ui,
    workspaces: &[WorkspaceState],
    active_workspace: usize,
    action: &mut Option<WorkspaceAction>,
    context_menu_open: &mut bool,
    create_workspace: &mut bool,
) {
    let (header_rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), WINDOW_TITLE_BAR_HEIGHT),
        Sense::hover(),
    );
    let mut header = ui.new_child(egui::UiBuilder::new().max_rect(header_rect).layout(
        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
    ));
    if compact_icon_button(&mut header, "+", "New workspace")
        .on_hover_text("New workspace")
        .clicked()
    {
        *create_workspace = true;
    }
    ui.add_space(6.0);

    for (index, workspace) in workspaces.iter().enumerate() {
        if let Some(next) = compact_workspace_item(
            ui,
            workspace,
            index,
            index == active_workspace,
            context_menu_open,
        ) {
            *action = Some(next);
        }
    }
}

fn compact_workspace_item(
    ui: &mut egui::Ui,
    workspace: &WorkspaceState,
    index: usize,
    active: bool,
    context_menu_open: &mut bool,
) -> Option<WorkspaceAction> {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 54.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            active,
            &workspace.model.name,
        )
    });

    if active || response.hovered() || response.context_menu_opened() {
        let selection_rect = egui::Rect::from_center_size(rect.center(), Vec2::new(40.0, 48.0));
        ui.painter().rect_filled(
            selection_rect,
            7.0,
            if active {
                Color32::from_rgb(20, 20, 20)
            } else {
                surface_hover()
            },
        );
    }

    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 17.0),
        Vec2::splat(22.0),
    );
    paint_workspace_icon(ui, icon_rect, workspace);
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 6.0),
        egui::Align2::CENTER_BOTTOM,
        compact_text(&workspace.model.name, 9),
        FontId::proportional(8.5),
        if active {
            text_primary()
        } else {
            text_secondary()
        },
    );

    let response = response.on_hover_text(format!(
        "{}\n{}",
        workspace.model.name,
        workspace.model.root_directory.display()
    ));
    let mut next = response.clicked().then_some(WorkspaceAction::Focus(index));
    if response.double_clicked() {
        next = Some(WorkspaceAction::Edit(
            workspace.model.id,
            workspace.model.name.clone(),
        ));
    }
    workspace_context_menu(&response, workspace, &mut next);
    *context_menu_open |= response.context_menu_opened();
    next
}

fn paint_workspace_icon(ui: &egui::Ui, rect: egui::Rect, workspace: &WorkspaceState) {
    let (background, foreground) = workspace_identity_colors(workspace.model.id);
    ui.painter().rect_filled(rect, 6.0, background);
    ui.painter().rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, foreground.gamma_multiply(0.18)),
        egui::StrokeKind::Inside,
    );
    let initial = workspace
        .model
        .name
        .chars()
        .find(char::is_ascii_alphanumeric)
        .map_or('W', |character| character.to_ascii_uppercase());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        FontId::proportional(11.0),
        foreground,
    );
    if workspace.any_running() {
        let status_center = egui::pos2(rect.right() - 3.0, rect.bottom() - 3.0);
        ui.painter()
            .circle_filled(status_center, 2.5, Color32::from_rgb(0x10, 0x7d, 0x32));
        ui.painter()
            .circle_stroke(status_center, 2.5, Stroke::new(1.0, surface_primary()));
    }
}

fn workspace_identity_colors(workspace_id: ade_core::WorkspaceId) -> (Color32, Color32) {
    const BACKGROUNDS: [(u8, u8, u8); 7] = [
        (0x00, 0x2f, 0x62),
        (0x5d, 0x0e, 0x17),
        (0x50, 0x28, 0x00),
        (0x00, 0x3a, 0x0e),
        (0x00, 0x3d, 0x34),
        (0x47, 0x18, 0x5e),
        (0x57, 0x10, 0x32),
    ];
    const FOREGROUNDS: [(u8, u8, u8); 7] = [
        (0xea, 0xf6, 0xff),
        (0xff, 0xe9, 0xed),
        (0xff, 0xf3, 0xd5),
        (0xd8, 0xff, 0xe4),
        (0xcb, 0xff, 0xf5),
        (0xfb, 0xec, 0xff),
        (0xff, 0xe9, 0xf4),
    ];
    let hash = workspace_id
        .to_string()
        .bytes()
        .fold(2_166_136_261_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        });
    let index = hash as usize % BACKGROUNDS.len();
    let background = BACKGROUNDS[index];
    let foreground = FOREGROUNDS[index];
    (
        Color32::from_rgb(background.0, background.1, background.2),
        Color32::from_rgb(foreground.0, foreground.1, foreground.2),
    )
}

fn workspace_row(
    ui: &mut egui::Ui,
    workspace: &WorkspaceState,
    index: usize,
    active: bool,
    context_menu_open: &mut bool,
) -> Option<WorkspaceAction> {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), SIDEBAR_ROW_HEIGHT),
        Sense::click(),
    );
    let response = response.on_hover_text(format!(
        "{}\n{}",
        workspace.model.name,
        workspace.model.root_directory.display()
    ));
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            active,
            &workspace.model.name,
        )
    });
    let fill = if active {
        surface_active()
    } else if response.hovered() || response.context_menu_opened() {
        surface_hover()
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 6.0, fill);

    let content = rect.shrink2(Vec2::new(8.0, 4.0));
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(content.left() + 12.0, content.center().y),
        Vec2::splat(24.0),
    );
    paint_workspace_icon(ui, icon_rect, workspace);

    let text_rect = egui::Rect::from_min_max(
        egui::pos2(content.left() + 32.0, content.top()),
        content.max,
    );
    let painter = ui.painter().with_clip_rect(text_rect);
    painter.text(
        egui::pos2(text_rect.left(), text_rect.top() + 8.5),
        egui::Align2::LEFT_CENTER,
        &workspace.model.name,
        FontId::proportional(12.5),
        text_primary(),
    );
    painter.text(
        egui::pos2(text_rect.left(), text_rect.top() + 24.5),
        egui::Align2::LEFT_CENTER,
        compact_path(&workspace.model.root_directory),
        FontId::proportional(10.0),
        text_secondary(),
    );

    let mut action = if response.clicked() {
        Some(WorkspaceAction::Focus(index))
    } else {
        None
    };
    if response.double_clicked() {
        action = Some(WorkspaceAction::Edit(
            workspace.model.id,
            workspace.model.name.clone(),
        ));
    }
    workspace_context_menu(&response, workspace, &mut action);
    *context_menu_open |= response.context_menu_opened();
    action
}

fn workspace_context_menu(
    response: &egui::Response,
    workspace: &WorkspaceState,
    action: &mut Option<WorkspaceAction>,
) {
    response.context_menu(|ui| {
        ui.set_width(160.0);
        if menu_item(ui, "Rename", text_primary(), false, 32.0).clicked() {
            *action = Some(WorkspaceAction::Edit(
                workspace.model.id,
                workspace.model.name.clone(),
            ));
            ui.close();
        }
        ui.separator();
        if menu_item(ui, "Delete", danger(), false, 32.0).clicked() {
            *action = Some(WorkspaceAction::Close(workspace.model.id));
            ui.close();
        }
    });
}

fn menu_item(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    selected: bool,
    height: f32,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, text));
    if selected || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            6.0,
            if selected {
                surface_active()
            } else {
                surface_hover()
            },
        );
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(if height < 36.0 { 13.0 } else { 14.0 }),
        color,
    );
    response
}

fn compact_icon_button(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 6.0, surface_hover());
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::proportional(16.0),
        text_primary(),
    );
    response
}

fn modal_primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(64.0, 32.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let fill = if response.is_pointer_button_down_on() {
        Color32::from_rgb(210, 210, 210)
    } else if response.hovered() || response.has_focus() {
        Color32::from_rgb(245, 245, 245)
    } else {
        text_primary()
    };
    ui.painter().rect(
        rect,
        6.0,
        fill,
        Stroke::new(1.0, Color32::WHITE),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(14.0),
        Color32::BLACK,
    );
    response
}

struct TerminalPane {
    id: PaneId,
    parser: vt100::Parser,
    status: SessionStatus,
    columns: u16,
    rows: u16,
    selection: Option<TerminalSelection>,
    reveal_started_at: Instant,
    close_started_at: Option<Instant>,
    close_request_sent: bool,
    cwd: PathBuf,
    git_status: Option<GitStatus>,
    git_refresh_pending: bool,
    git_last_refreshed: Option<Instant>,
    git_results: Receiver<(PathBuf, Option<GitStatus>)>,
    git_result_sender: Sender<(PathBuf, Option<GitStatus>)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GitStatus {
    branch: String,
    changed_files: usize,
    additions: usize,
    deletions: usize,
}

#[derive(Clone, Copy)]
struct CellPosition {
    row: u16,
    column: u16,
}

#[derive(Clone, Copy)]
struct TerminalSelection {
    start: CellPosition,
    end: CellPosition,
}

impl TerminalPane {
    fn new(metadata: &PaneSnapshot) -> Self {
        let (git_result_sender, git_results) = unbounded();
        Self {
            id: metadata.id,
            parser: vt100::Parser::new(metadata.rows, metadata.cols, SCROLLBACK_LINES),
            status: metadata.status.clone(),
            columns: metadata.cols,
            rows: metadata.rows,
            selection: None,
            reveal_started_at: Instant::now(),
            close_started_at: None,
            close_request_sent: false,
            cwd: metadata.cwd.clone(),
            git_status: None,
            git_refresh_pending: false,
            git_last_refreshed: None,
            git_results,
            git_result_sender,
        }
    }

    fn reveal_progress(&self) -> f32 {
        let elapsed = self.reveal_started_at.elapsed().as_secs_f32();
        (elapsed / TERMINAL_REVEAL_DURATION.as_secs_f32()).clamp(0.0, 1.0)
    }

    fn close_progress(&self) -> Option<f32> {
        self.close_started_at.map(|started_at| {
            (started_at.elapsed().as_secs_f32() / TERMINAL_CLOSE_DURATION.as_secs_f32())
                .clamp(0.0, 1.0)
        })
    }

    fn update_metadata(&mut self, metadata: &PaneSnapshot) {
        self.status = metadata.status.clone();
        if self.cwd != metadata.cwd {
            self.cwd.clone_from(&metadata.cwd);
            self.git_status = None;
            self.git_last_refreshed = None;
            self.git_refresh_pending = false;
        }
    }

    fn refresh_git_status(&mut self) {
        while let Ok((cwd, status)) = self.git_results.try_recv() {
            if cwd == self.cwd {
                self.git_status = status;
                self.git_last_refreshed = Some(Instant::now());
                self.git_refresh_pending = false;
            }
        }
        let due = self
            .git_last_refreshed
            .is_none_or(|last| last.elapsed() >= GIT_REFRESH_INTERVAL);
        if self.git_refresh_pending || !due {
            return;
        }
        self.git_refresh_pending = true;
        let cwd = self.cwd.clone();
        let sender = self.git_result_sender.clone();
        let _ = thread::Builder::new()
            .name(format!("ade-git-status-{}", self.id))
            .spawn(move || {
                let status = read_git_status(&cwd);
                let _ = sender.send((cwd, status));
            });
    }

    fn process_output(&mut self, data: &[u8]) {
        self.parser.process(data);
    }

    fn send(&self, requests: &Sender<ClientRequest>, bytes: impl Into<Vec<u8>>) {
        let _ = requests.send(ClientRequest::Input {
            pane_id: self.id,
            data: bytes.into(),
        });
    }

    fn paste_bytes(&self, text: &str) -> Vec<u8> {
        let normalized = text.replace("\r\n", "\n").replace('\0', "");
        if self.parser.screen().bracketed_paste() {
            format!("\x1b[200~{normalized}\x1b[201~").into_bytes()
        } else {
            normalized.into_bytes()
        }
    }

    fn selected_text(&self) -> Option<String> {
        let selection = self.selection?;
        let (start, end) = ordered_selection(selection, self.columns);
        let end_column = end.column.saturating_add(1).min(self.columns);
        Some(
            self.parser
                .screen()
                .contents_between(start.row, start.column, end.row, end_column)
                .replace(TERMINAL_DIVIDER_MARKER, ""),
        )
    }

    fn resize(&mut self, requests: &Sender<ClientRequest>, columns: u16, rows: u16) {
        if columns == self.columns && rows == self.rows {
            return;
        }
        self.parser.screen_mut().set_size(rows, columns);
        self.columns = columns;
        self.rows = rows;
        let _ = requests.send(ClientRequest::Resize {
            pane_id: self.id,
            cols: columns,
            rows,
        });
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn render_layout(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    node: &mut LayoutNode,
    panes: &mut HashMap<PaneId, TerminalPane>,
    active_pane: &mut Option<PaneId>,
    requests: &Sender<ClientRequest>,
    accept_input: bool,
    path: &str,
) -> bool {
    match node {
        LayoutNode::Empty => false,
        LayoutNode::Pane { pane_id } => {
            if let Some(pane) = panes.get_mut(pane_id) {
                let reveal = egui::emath::easing::cubic_out(pane.reveal_progress());
                if reveal < 1.0 {
                    ui.ctx().request_repaint();
                }
                let visual_rect = rect.translate(Vec2::new(
                    0.0,
                    egui::lerp(TERMINAL_REVEAL_OFFSET..=0.0, reveal),
                ));
                let mut pane_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                pane_ui.set_opacity(egui::lerp(0.72..=1.0, reveal));
                terminal_pane_ui(
                    &mut pane_ui,
                    visual_rect,
                    pane,
                    Some(*pane_id) == *active_pane,
                    active_pane,
                    requests,
                    accept_input,
                );
            }
            false
        }
        LayoutNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let pixels_per_point = ui.ctx().pixels_per_point();
            let available = match axis {
                SplitAxis::Columns => rect.width() - DIVIDER_SIZE,
                SplitAxis::Rows => rect.height() - DIVIDER_SIZE,
            };
            let first_minimum = minimum_layout_extent(first, *axis);
            let second_minimum = minimum_layout_extent(second, *axis);
            let first_dividers = internal_divider_extent(first, *axis);
            let second_dividers = internal_divider_extent(second, *axis);
            let allocation_ratio =
                allocation_ratio_for_layout(*ratio, available, first_dividers, second_dividers);
            let ratio_value =
                constrained_split_ratio(allocation_ratio, available, first_minimum, second_minimum);
            // Minimum pane sizes constrain only this frame. Persisting the clamp would leave an
            // equal grid uneven after a temporarily narrow window is enlarged again.
            let mut changed = false;
            let (first_rect, divider_rect, second_rect) = match axis {
                SplitAxis::Columns => {
                    let split_x =
                        snap_to_pixel(rect.left() + available * ratio_value, pixels_per_point);
                    (
                        egui::Rect::from_min_max(rect.min, egui::pos2(split_x, rect.bottom())),
                        egui::Rect::from_min_max(
                            egui::pos2(split_x, rect.top()),
                            egui::pos2(split_x + DIVIDER_SIZE, rect.bottom()),
                        ),
                        egui::Rect::from_min_max(
                            egui::pos2(split_x + DIVIDER_SIZE, rect.top()),
                            rect.max,
                        ),
                    )
                }
                SplitAxis::Rows => {
                    let split_y =
                        snap_to_pixel(rect.top() + available * ratio_value, pixels_per_point);
                    (
                        egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), split_y)),
                        egui::Rect::from_min_max(
                            egui::pos2(rect.left(), split_y),
                            egui::pos2(rect.right(), split_y + DIVIDER_SIZE),
                        ),
                        egui::Rect::from_min_max(
                            egui::pos2(rect.left(), split_y + DIVIDER_SIZE),
                            rect.max,
                        ),
                    )
                }
            };

            let response = ui.interact(
                divider_rect.expand(3.0),
                egui::Id::new(("split-divider", path)),
                Sense::drag(),
            );
            if response.dragged() {
                let pointer = response
                    .interact_pointer_pos()
                    .unwrap_or(divider_rect.center());
                let requested_allocation = match axis {
                    SplitAxis::Columns => constrained_split_ratio(
                        (pointer.x - rect.left()) / available,
                        available,
                        first_minimum,
                        second_minimum,
                    ),
                    SplitAxis::Rows => constrained_split_ratio(
                        (pointer.y - rect.top()) / available,
                        available,
                        first_minimum,
                        second_minimum,
                    ),
                };
                *ratio = layout_ratio_for_allocation(
                    requested_allocation,
                    available,
                    first_dividers,
                    second_dividers,
                );
                changed = true;
            }
            let divider_color = if response.hovered() || response.dragged() {
                Color32::from_rgb(0, 110, 254)
            } else {
                border()
            };
            let line_width = 1.0 / pixels_per_point;
            let visual_divider = match axis {
                SplitAxis::Columns => {
                    let left = snap_to_pixel(divider_rect.center().x, pixels_per_point);
                    egui::Rect::from_min_max(
                        egui::pos2(left, divider_rect.top()),
                        egui::pos2(left + line_width, divider_rect.bottom()),
                    )
                }
                SplitAxis::Rows => {
                    let top = snap_to_pixel(divider_rect.center().y, pixels_per_point);
                    egui::Rect::from_min_max(
                        egui::pos2(divider_rect.left(), top),
                        egui::pos2(divider_rect.right(), top + line_width),
                    )
                }
            };
            ui.painter().rect_filled(visual_divider, 0.0, divider_color);

            let first_changed = render_layout(
                ui,
                first_rect,
                first,
                panes,
                active_pane,
                requests,
                accept_input,
                &format!("{path}.0"),
            );
            let second_changed = render_layout(
                ui,
                second_rect,
                second,
                panes,
                active_pane,
                requests,
                accept_input,
                &format!("{path}.1"),
            );
            changed || first_changed || second_changed
        }
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]
fn terminal_pane_ui(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    pane: &mut TerminalPane,
    active: bool,
    active_pane: &mut Option<PaneId>,
    requests: &Sender<ClientRequest>,
    accept_input: bool,
) {
    ui.painter().rect_filled(rect, 0.0, terminal_background());
    pane.refresh_git_status();
    let footer_top = (rect.bottom() - TERMINAL_FOOTER_HEIGHT).max(rect.top());
    let footer_rect = egui::Rect::from_min_max(egui::pos2(rect.left(), footer_top), rect.max);
    let content_min = egui::pos2(
        (rect.left() + TERMINAL_SIDE_PADDING).min(rect.right()),
        (rect.top() + TERMINAL_BOTTOM_PADDING).min(rect.bottom()),
    );
    let content_rect = egui::Rect::from_min_max(
        content_min,
        egui::pos2(
            (rect.right() - TERMINAL_SIDE_PADDING).max(content_min.x),
            (footer_top - TERMINAL_BOTTOM_PADDING).max(content_min.y),
        ),
    );
    let font_id = FontId::new(TERMINAL_FONT_SIZE, FontFamily::Monospace);
    let (cell_width, measured_height) = ui.fonts_mut(|fonts| {
        (
            fonts.glyph_width(&font_id, 'M').max(1.0),
            fonts.row_height(&font_id).max(1.0),
        )
    });
    let pixels_per_point = ui.ctx().pixels_per_point();
    let cell_height = ((measured_height * pixels_per_point / 2.0).ceil() * 2.0) / pixels_per_point;
    let columns = cells_for_pixels(content_rect.width(), cell_width);
    let rows = cells_for_pixels(content_rect.height(), cell_height);
    pane.resize(requests, columns, rows);

    let screen = pane.parser.screen();
    let (cursor_row, cursor_column) = screen.cursor_position();
    let divider_rows = terminal_divider_rows(screen);
    let bottom_anchored = !screen.alternate_screen() && screen.scrollback() == 0;
    let grid_origin = egui::pos2(
        snap_to_pixel(content_rect.left(), pixels_per_point),
        if bottom_anchored {
            snap_to_pixel(
                content_rect.bottom() - f32::from(cursor_row.saturating_add(1)) * cell_height,
                pixels_per_point,
            )
        } else {
            snap_to_pixel(content_rect.top(), pixels_per_point)
        },
    );
    let current_block_top = if bottom_anchored {
        divider_rows.last().map(|row| {
            snap_to_pixel(
                grid_origin.y + f32::from(row.saturating_add(1)) * cell_height
                    - TERMINAL_DIVIDER_OFFSET,
                pixels_per_point,
            )
        })
    } else {
        None
    };

    if bottom_anchored {
        let fallback_dock_top =
            grid_origin.y + f32::from(cursor_row) * cell_height - TERMINAL_DIVIDER_OFFSET;
        let dock_top = current_block_top
            .map_or(fallback_dock_top, |top| top + 2.0 * cell_height)
            .max(content_rect.top());
        let dock_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left(), dock_top),
            egui::pos2(rect.right(), footer_top),
        );
        ui.painter()
            .rect_filled(dock_rect, 0.0, Color32::from_rgb(10, 10, 10));
        if current_block_top.is_none() {
            ui.painter().hline(
                dock_rect.x_range(),
                dock_rect.top(),
                Stroke::new(1.0, terminal_divider_color()),
            );
        }
        if active {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(rect.left(), dock_top + 7.0),
                    Vec2::new(2.0, cell_height),
                ),
                1.0,
                Color32::from_rgb(0, 110, 254),
            );
        }
    }

    paint_terminal_footer(ui, footer_rect, active);

    let job = terminal_layout_job(&pane.parser, pane.selection, cell_height);
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    ui.painter()
        .with_clip_rect(content_rect)
        .galley(grid_origin, galley, Color32::WHITE);
    let divider_stroke = Stroke::new(1.0, terminal_divider_color());
    let divider_painter = ui.painter().with_clip_rect(rect);
    for (index, row) in divider_rows.iter().copied().enumerate() {
        let y = snap_to_pixel(
            grid_origin.y + f32::from(row.saturating_add(1)) * cell_height
                - TERMINAL_DIVIDER_OFFSET,
            pixels_per_point,
        );
        if (rect.top()..=rect.bottom()).contains(&y) {
            let current = index + 1 == divider_rows.len() && bottom_anchored;
            if !current {
                divider_painter.hline(rect.x_range(), y, divider_stroke);
            }
        }
    }
    if let Some(header_top) = current_block_top {
        let header_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left(), header_top),
            egui::pos2(rect.right(), header_top + 2.0 * cell_height),
        );
        paint_command_header(ui, header_rect, &pane.cwd, pane.git_status.as_ref(), active);
    }
    let response = ui.interact(
        content_rect,
        egui::Id::new(("terminal-content", pane.id)),
        Sense::click_and_drag(),
    );
    if pane.close_started_at.is_none() && (response.clicked() || response.drag_started()) {
        *active_pane = Some(pane.id);
        let _ = requests.send(ClientRequest::FocusPane { pane_id: pane.id });
        response.request_focus();
    }
    if pane.close_started_at.is_none() && response.clicked() {
        pane.selection = None;
    }
    if pane.close_started_at.is_none() && response.drag_started() {
        if let Some(position) = response.interact_pointer_pos().and_then(|pointer| {
            cell_at_pointer(
                pointer,
                content_rect,
                grid_origin,
                pane.rows,
                pane.columns,
                cell_width,
                cell_height,
            )
        }) {
            pane.selection = Some(TerminalSelection {
                start: position,
                end: position,
            });
        }
    } else if response.dragged()
        && let Some(position) = response.interact_pointer_pos().and_then(|pointer| {
            cell_at_pointer(
                pointer,
                content_rect,
                grid_origin,
                pane.rows,
                pane.columns,
                cell_width,
                cell_height,
            )
        })
        && let Some(selection) = &mut pane.selection
    {
        selection.end = position;
    }

    if active && !screen.hide_cursor() {
        let cursor_min = egui::pos2(
            grid_origin.x + f32::from(cursor_column) * cell_width,
            grid_origin.y + f32::from(cursor_row) * cell_height + 2.0,
        );
        let cursor_rect =
            egui::Rect::from_min_size(cursor_min, Vec2::new(2.0, (cell_height - 4.0).max(2.0)))
                .intersect(content_rect);
        ui.painter().rect_filled(cursor_rect, 1.0, text_primary());
    }

    if response.hovered() {
        let scroll = ui.ctx().input(|input| input.smooth_scroll_delta().y);
        if scroll.abs() > f32::EPSILON {
            let current = pane.parser.screen().scrollback();
            let lines = scroll_lines(scroll);
            let next = if scroll > 0.0 {
                current.saturating_add(lines)
            } else {
                current.saturating_sub(lines)
            };
            pane.parser.screen_mut().set_scrollback(next);
        }
    }

    if active && accept_input && pane.close_started_at.is_none() {
        forward_terminal_input(ui.ctx(), pane, requests);
    }

    if let Some(progress) = pane.close_progress() {
        let pulse = (progress * std::f32::consts::PI).sin();
        let alpha = (210.0 + 45.0 * pulse) as u8;
        let red = Color32::from_rgba_unmultiplied(255, 105, 105, alpha);
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            0.0,
            Stroke::new(1.0, red),
            egui::StrokeKind::Inside,
        );
    }
}

fn paint_terminal_footer(ui: &mut egui::Ui, rect: egui::Rect, active: bool) {
    ui.painter().hline(
        rect.x_range(),
        rect.top(),
        Stroke::new(1.0, Color32::from_rgb(38, 38, 38)),
    );

    let key_color = if active {
        Color32::from_rgb(190, 190, 190)
    } else {
        Color32::from_rgb(105, 105, 105)
    };
    let label_color = if active {
        Color32::from_rgb(125, 125, 125)
    } else {
        Color32::from_rgb(76, 76, 76)
    };
    let baseline = egui::pos2(rect.left() + TERMINAL_SIDE_PADDING, rect.center().y);
    ui.painter().text(
        baseline,
        egui::Align2::LEFT_CENTER,
        "Ctrl+Shift+P",
        FontId::new(11.0, FontFamily::Monospace),
        key_color,
    );
    ui.painter().text(
        baseline + Vec2::new(90.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "Command palette",
        FontId::proportional(12.0),
        label_color,
    );
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]
fn paint_command_header(
    ui: &mut egui::Ui,
    header_rect: egui::Rect,
    cwd: &Path,
    git: Option<&GitStatus>,
    active: bool,
) {
    const CHIP_HEIGHT: f32 = 22.0;
    const GAP: f32 = 7.0;
    const ICON_WIDTH: f32 = 12.0;
    const ICON_TEXT_GAP: f32 = 6.0;
    const STAT_GAP: f32 = 16.0;
    let clip = ui.painter().with_clip_rect(header_rect.expand(1.0));
    let separator = Stroke::new(1.0, Color32::from_rgb(38, 38, 38));
    clip.hline(header_rect.x_range(), header_rect.top(), separator);
    clip.hline(header_rect.x_range(), header_rect.bottom(), separator);

    let center_y = header_rect.center().y;
    let left = header_rect.left() + TERMINAL_SIDE_PADDING;
    let right = header_rect.right() - TERMINAL_SIDE_PADDING;
    let available = (right - left).max(0.0);
    let max_path_width = (available * if git.is_some() { 0.48 } else { 0.72 })
        .clamp(96.0, 360.0)
        .min(available);
    let max_path_chars = ((max_path_width - 34.0) / 7.5).max(4.0) as usize;
    let path_label = compact_header_path(cwd, max_path_chars);
    let font = FontId::new(12.0, FontFamily::Monospace);
    let path_text_width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(path_label.clone(), font.clone(), text_primary())
            .size()
            .x
    });
    let path_width = (path_text_width + 34.0).min(max_path_width);
    let path_rect = egui::Rect::from_min_size(
        egui::pos2(left, center_y - CHIP_HEIGHT / 2.0),
        Vec2::new(path_width, CHIP_HEIGHT),
    );
    paint_header_chip(
        &clip,
        path_rect,
        Color32::BLACK,
        if active {
            Color32::from_rgb(72, 72, 72)
        } else {
            Color32::from_rgb(46, 46, 46)
        },
    );
    paint_folder_icon(
        &clip,
        egui::pos2(path_rect.left() + 8.0, path_rect.center().y),
        if active {
            text_primary()
        } else {
            text_secondary()
        },
    );
    clip.text(
        egui::pos2(path_rect.left() + 25.0, path_rect.center().y),
        egui::Align2::LEFT_CENTER,
        path_label,
        font.clone(),
        if active {
            text_primary()
        } else {
            text_secondary()
        },
    );

    let Some(git) = git else {
        return;
    };
    let mut x = path_rect.right() + GAP;
    let remaining = right - x;
    if remaining < 86.0 {
        return;
    }
    let branch_label = compact_text(&git.branch, if remaining < 220.0 { 10 } else { 20 });
    let branch_text_width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(
                branch_label.clone(),
                font.clone(),
                Color32::from_rgb(151, 211, 142),
            )
            .size()
            .x
    });
    let branch_width = (branch_text_width + 31.0).min((remaining * 0.6).max(78.0));
    let branch_rect = egui::Rect::from_min_size(
        egui::pos2(x, center_y - CHIP_HEIGHT / 2.0),
        Vec2::new(branch_width, CHIP_HEIGHT),
    );
    paint_header_chip(
        &clip,
        branch_rect,
        Color32::from_rgb(8, 18, 10),
        Color32::from_rgb(35, 65, 39),
    );
    let git_green = Color32::from_rgb(151, 211, 142);
    paint_git_branch_icon(
        &clip,
        egui::pos2(branch_rect.left() + 8.0, branch_rect.center().y),
        git_green,
    );
    clip.text(
        egui::pos2(branch_rect.left() + 23.0, branch_rect.center().y),
        egui::Align2::LEFT_CENTER,
        branch_label,
        font.clone(),
        git_green,
    );
    x = branch_rect.right() + GAP;

    let stats_width = right - x;
    let count_label = git.changed_files.to_string();
    let addition_label = format!("+{}", git.additions);
    let deletion_label = format!("-{}", git.deletions);
    let (count_width, addition_width, deletion_width) = ui.fonts_mut(|fonts| {
        (
            fonts
                .layout_no_wrap(
                    count_label.clone(),
                    font.clone(),
                    Color32::from_rgb(175, 175, 175),
                )
                .size()
                .x,
            fonts
                .layout_no_wrap(
                    addition_label.clone(),
                    font.clone(),
                    Color32::from_rgb(82, 196, 92),
                )
                .size()
                .x,
            fonts
                .layout_no_wrap(
                    deletion_label.clone(),
                    font.clone(),
                    Color32::from_rgb(238, 91, 91),
                )
                .size()
                .x,
        )
    });
    let count_group_width = ICON_WIDTH + ICON_TEXT_GAP + count_width;
    if stats_width < count_group_width {
        return;
    }
    paint_file_icon(
        &clip,
        egui::pos2(x, center_y),
        Color32::from_rgb(145, 145, 145),
    );
    x += ICON_WIDTH + ICON_TEXT_GAP;
    clip.text(
        egui::pos2(x, center_y),
        egui::Align2::LEFT_CENTER,
        count_label,
        font.clone(),
        Color32::from_rgb(175, 175, 175),
    );
    x += count_width;
    let additions_fit = count_group_width + STAT_GAP + addition_width <= stats_width;
    if additions_fit {
        x += STAT_GAP;
        clip.text(
            egui::pos2(x, center_y),
            egui::Align2::LEFT_CENTER,
            addition_label,
            font.clone(),
            Color32::from_rgb(82, 196, 92),
        );
        x += addition_width;
    }
    let deletions_fit =
        count_group_width + STAT_GAP + addition_width + STAT_GAP + deletion_width <= stats_width;
    if additions_fit && deletions_fit {
        x += STAT_GAP;
        clip.text(
            egui::pos2(x, center_y),
            egui::Align2::LEFT_CENTER,
            deletion_label,
            font,
            Color32::from_rgb(238, 91, 91),
        );
    }
}

fn paint_header_chip(painter: &egui::Painter, rect: egui::Rect, fill: Color32, stroke: Color32) {
    painter.rect(
        rect,
        4.0,
        fill,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
}

fn paint_folder_icon(painter: &egui::Painter, origin: egui::Pos2, color: Color32) {
    let points = [
        origin + Vec2::new(0.0, -5.0),
        origin + Vec2::new(5.0, -5.0),
        origin + Vec2::new(7.0, -2.5),
        origin + Vec2::new(13.0, -2.5),
        origin + Vec2::new(13.0, 5.0),
        origin + Vec2::new(0.0, 5.0),
        origin + Vec2::new(0.0, -5.0),
    ];
    painter.add(egui::Shape::line(points.to_vec(), Stroke::new(1.4, color)));
}

fn paint_git_branch_icon(painter: &egui::Painter, origin: egui::Pos2, color: Color32) {
    let stroke = Stroke::new(1.25, color);
    painter.circle_stroke(origin + Vec2::new(2.5, -4.5), 2.0, stroke);
    painter.circle_stroke(origin + Vec2::new(10.5, -4.5), 2.0, stroke);
    painter.circle_stroke(origin + Vec2::new(2.5, 5.0), 2.0, stroke);
    painter.line_segment(
        [origin + Vec2::new(2.5, -2.5), origin + Vec2::new(2.5, 3.0)],
        stroke,
    );
    painter.add(egui::Shape::line(
        vec![
            origin + Vec2::new(10.5, -2.5),
            origin + Vec2::new(10.5, 0.0),
            origin + Vec2::new(5.0, 2.0),
        ],
        stroke,
    ));
}

fn paint_file_icon(painter: &egui::Painter, origin: egui::Pos2, color: Color32) {
    let stroke = Stroke::new(1.25, color);
    painter.add(egui::Shape::line(
        vec![
            origin + Vec2::new(1.0, -6.0),
            origin + Vec2::new(7.0, -6.0),
            origin + Vec2::new(11.0, -2.0),
            origin + Vec2::new(11.0, 6.0),
            origin + Vec2::new(1.0, 6.0),
            origin + Vec2::new(1.0, -6.0),
        ],
        stroke,
    ));
    painter.line_segment(
        [origin + Vec2::new(7.0, -6.0), origin + Vec2::new(7.0, -2.0)],
        stroke,
    );
    painter.line_segment(
        [
            origin + Vec2::new(7.0, -2.0),
            origin + Vec2::new(11.0, -2.0),
        ],
        stroke,
    );
}

fn terminal_layout_job(
    parser: &vt100::Parser,
    selection: Option<TerminalSelection>,
    cell_height: f32,
) -> LayoutJob {
    let screen = parser.screen();
    let (rows, columns) = screen.size();
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    job.keep_trailing_whitespace = true;
    job.round_output_to_gui = false;

    for row in 0..rows {
        let divider_row = is_terminal_divider_row(screen, row);
        for column in 0..columns {
            let Some(cell) = screen.cell(row, column) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let text = if divider_row {
                " "
            } else if cell.has_contents() {
                cell.contents()
            } else {
                " "
            };
            let mut foreground = terminal_color(cell.fgcolor(), false);
            let mut background = terminal_color(cell.bgcolor(), true);
            let selected = selection.is_some_and(|selection| {
                let (start, end) = ordered_selection(selection, columns);
                let index = usize::from(row) * usize::from(columns) + usize::from(column);
                let start_index =
                    usize::from(start.row) * usize::from(columns) + usize::from(start.column);
                let end_index =
                    usize::from(end.row) * usize::from(columns) + usize::from(end.column);
                (start_index..=end_index).contains(&index)
            });
            if cell.inverse() {
                std::mem::swap(&mut foreground, &mut background);
            }
            if selected {
                background = Color32::from_rgb(0, 74, 168);
            }
            if cell.dim() {
                foreground = foreground.gamma_multiply(0.65);
            }
            job.append(
                text,
                0.0,
                TextFormat {
                    font_id: FontId::new(TERMINAL_FONT_SIZE, FontFamily::Monospace),
                    line_height: Some(cell_height),
                    color: foreground,
                    background,
                    expand_bg: 0.5,
                    italics: cell.italic(),
                    underline: if cell.underline() {
                        Stroke::new(1.0, foreground)
                    } else {
                        Stroke::NONE
                    },
                    ..Default::default()
                },
            );
        }
        if row + 1 < rows {
            job.append(
                "\n",
                0.0,
                TextFormat {
                    font_id: FontId::new(TERMINAL_FONT_SIZE, FontFamily::Monospace),
                    line_height: Some(cell_height),
                    color: Color32::WHITE,
                    ..Default::default()
                },
            );
        }
    }
    job
}

fn terminal_divider_rows(screen: &vt100::Screen) -> Vec<u16> {
    let (rows, _) = screen.size();
    (0..rows)
        .filter(|&row| is_terminal_divider_row(screen, row))
        .collect()
}

fn is_terminal_divider_row(screen: &vt100::Screen, row: u16) -> bool {
    let (_, columns) = screen.size();
    let marker_length = u16::try_from(TERMINAL_DIVIDER_MARKER.len()).unwrap_or(u16::MAX);
    if columns < marker_length {
        return false;
    }
    TERMINAL_DIVIDER_MARKER
        .chars()
        .enumerate()
        .all(|(column, expected)| {
            screen
                .cell(row, u16::try_from(column).unwrap_or(u16::MAX))
                .is_some_and(|cell| cell.contents() == expected.to_string())
        })
}

fn forward_terminal_input(
    context: &egui::Context,
    pane: &TerminalPane,
    requests: &Sender<ClientRequest>,
) {
    let events = context.input(|input| input.events.clone());
    for event in events {
        match event {
            egui::Event::Text(text) if !text.is_empty() => {
                let mut bytes = text.into_bytes();
                if context.input(|input| input.modifiers.alt) {
                    bytes.insert(0, 0x1b);
                }
                pane.send(requests, bytes);
            }
            egui::Event::Paste(text) => {
                pane.send(requests, pane.paste_bytes(&text));
            }
            egui::Event::Key {
                key,
                pressed: true,
                repeat: _,
                modifiers,
                ..
            } => {
                if let Some(bytes) = encode_key(key, modifiers, pane.parser.screen()) {
                    pane.send(requests, bytes);
                }
            }
            _ => {}
        }
    }
}

fn encode_key(key: Key, modifiers: Modifiers, screen: &vt100::Screen) -> Option<Vec<u8>> {
    if modifiers.ctrl && !modifiers.shift {
        let control = match key {
            Key::A => 0x01,
            Key::B => 0x02,
            Key::C => 0x03,
            Key::D => 0x04,
            Key::E => 0x05,
            Key::F => 0x06,
            Key::G => 0x07,
            Key::H => 0x08,
            Key::I => 0x09,
            Key::J => 0x0a,
            Key::K => 0x0b,
            Key::L => 0x0c,
            Key::M => 0x0d,
            Key::N => 0x0e,
            Key::O => 0x0f,
            Key::P => 0x10,
            Key::Q => 0x11,
            Key::R => 0x12,
            Key::S => 0x13,
            Key::T => 0x14,
            Key::U => 0x15,
            Key::V => 0x16,
            Key::W => 0x17,
            Key::X => 0x18,
            Key::Y => 0x19,
            Key::Z => 0x1a,
            _ => return None,
        };
        return Some(vec![control]);
    }

    if matches!(
        key,
        Key::ArrowUp | Key::ArrowDown | Key::ArrowRight | Key::ArrowLeft
    ) && (modifiers.shift || modifiers.alt || modifiers.ctrl)
    {
        let modifier = 1
            + u8::from(modifiers.shift)
            + 2 * u8::from(modifiers.alt)
            + 4 * u8::from(modifiers.ctrl);
        let suffix = match key {
            Key::ArrowUp => 'A',
            Key::ArrowDown => 'B',
            Key::ArrowRight => 'C',
            Key::ArrowLeft => 'D',
            _ => unreachable!(),
        };
        return Some(format!("\x1b[1;{modifier}{suffix}").into_bytes());
    }

    let application_cursor = screen.application_cursor();
    let bytes: &[u8] = match key {
        Key::Enter => b"\r",
        Key::Tab if modifiers.shift => b"\x1b[Z",
        Key::Tab => b"\t",
        Key::Backspace => b"\x7f",
        Key::Escape => b"\x1b",
        Key::ArrowUp if application_cursor => b"\x1bOA",
        Key::ArrowDown if application_cursor => b"\x1bOB",
        Key::ArrowRight if application_cursor => b"\x1bOC",
        Key::ArrowLeft if application_cursor => b"\x1bOD",
        Key::ArrowUp => b"\x1b[A",
        Key::ArrowDown => b"\x1b[B",
        Key::ArrowRight => b"\x1b[C",
        Key::ArrowLeft => b"\x1b[D",
        Key::Home => b"\x1b[H",
        Key::End => b"\x1b[F",
        Key::Delete => b"\x1b[3~",
        Key::Insert => b"\x1b[2~",
        Key::PageUp => b"\x1b[5~",
        Key::PageDown => b"\x1b[6~",
        Key::F1 => b"\x1bOP",
        Key::F2 => b"\x1bOQ",
        Key::F3 => b"\x1bOR",
        Key::F4 => b"\x1bOS",
        Key::F5 => b"\x1b[15~",
        Key::F6 => b"\x1b[17~",
        Key::F7 => b"\x1b[18~",
        Key::F8 => b"\x1b[19~",
        Key::F9 => b"\x1b[20~",
        Key::F10 => b"\x1b[21~",
        Key::F11 => b"\x1b[23~",
        Key::F12 => b"\x1b[24~",
        _ => return None,
    };
    Some(bytes.to_vec())
}

fn shortcut(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::CTRL | Modifiers::SHIFT, key)
}

fn configure_style(context: &egui::Context) {
    configure_fonts(context);
    let mut style = (*context.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = terminal_background();
    style.visuals.window_fill = surface_primary();
    style.visuals.window_stroke = Stroke::new(1.0, border());
    style.visuals.window_corner_radius = 12.0.into();
    style.visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(180),
    };
    style.visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border());
    style.visuals.widgets.inactive.corner_radius = 6.0.into();
    style.visuals.widgets.hovered.bg_fill = surface_hover();
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, border_hover());
    style.visuals.widgets.hovered.corner_radius = 6.0.into();
    style.visuals.widgets.active.bg_fill = surface_active();
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, text_disabled());
    style.visuals.widgets.active.corner_radius = 6.0.into();
    style.visuals.selection.bg_fill = Color32::from_rgb(0, 110, 254);
    style.visuals.selection.stroke = Stroke::new(1.0, text_primary());
    style.visuals.hyperlink_color = Color32::from_rgb(71, 168, 255);
    style.visuals.override_text_color = Some(text_primary());
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(10.0, 6.0);
    style.spacing.interact_size = Vec2::new(40.0, 32.0);
    style.spacing.menu_margin = egui::Margin::same(6);
    style.text_styles.insert(
        egui::TextStyle::Body,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        FontId::new(12.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        FontId::new(20.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        FontId::new(14.0, FontFamily::Monospace),
    );
    context.set_style_of(egui::Theme::Dark, style);
}

fn configure_fonts(context: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "geist-sans".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/Geist-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "geist-mono".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/GeistMono-Regular.ttf"
        ))),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "geist-sans".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "geist-mono".to_owned());
    let candidates = [
        ("ade-console", r"C:\Windows\Fonts\CascadiaMono.ttf", true),
        ("ade-consolas", r"C:\Windows\Fonts\consola.ttf", true),
        ("ade-segoe", r"C:\Windows\Fonts\segoeui.ttf", false),
        ("ade-symbols", r"C:\Windows\Fonts\seguisym.ttf", false),
    ];
    for (name, path, monospace_primary) in candidates {
        let Ok(data) = std::fs::read(path) else {
            continue;
        };
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(egui::FontData::from_owned(data)));
        if monospace_primary {
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .insert(0, name.to_owned());
        } else {
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .push(name.to_owned());
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .push(name.to_owned());
        }
    }
    context.set_fonts(fonts);
}

fn terminal_background() -> Color32 {
    Color32::BLACK
}

fn surface_primary() -> Color32 {
    Color32::BLACK
}

fn surface_hover() -> Color32 {
    Color32::from_rgb(26, 26, 26)
}

fn surface_active() -> Color32 {
    Color32::from_rgb(31, 31, 31)
}

fn text_primary() -> Color32 {
    Color32::from_rgb(237, 237, 237)
}

fn text_secondary() -> Color32 {
    Color32::from_rgb(160, 160, 160)
}

fn text_disabled() -> Color32 {
    Color32::from_rgb(143, 143, 143)
}

fn border() -> Color32 {
    Color32::from_rgb(46, 46, 46)
}

fn border_hover() -> Color32 {
    Color32::from_rgb(69, 69, 69)
}

fn terminal_divider_color() -> Color32 {
    Color32::from_rgb(64, 64, 64)
}

fn danger() -> Color32 {
    Color32::from_rgb(226, 22, 42)
}

fn ordered_selection(selection: TerminalSelection, columns: u16) -> (CellPosition, CellPosition) {
    let index = |position: CellPosition| {
        usize::from(position.row) * usize::from(columns) + usize::from(position.column)
    };
    if index(selection.start) <= index(selection.end) {
        (selection.start, selection.end)
    } else {
        (selection.end, selection.start)
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn cell_at_pointer(
    pointer: egui::Pos2,
    content_rect: egui::Rect,
    grid_origin: egui::Pos2,
    rows: u16,
    columns: u16,
    cell_width: f32,
    cell_height: f32,
) -> Option<CellPosition> {
    if !content_rect.contains(pointer) {
        return None;
    }
    let column = ((pointer.x - grid_origin.x) / cell_width)
        .floor()
        .clamp(0.0, f32::from(columns.saturating_sub(1))) as u16;
    let row_value = ((pointer.y - grid_origin.y) / cell_height).floor();
    if row_value < 0.0 || row_value >= f32::from(rows) {
        return None;
    }
    let row = row_value
        .floor()
        .clamp(0.0, f32::from(rows.saturating_sub(1))) as u16;
    Some(CellPosition { row, column })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn cells_for_pixels(pixels: f32, cell_size: f32) -> u16 {
    // The value is explicitly clamped to the range accepted by ConPTY before conversion.
    (pixels / cell_size).floor().clamp(2.0, f32::from(i16::MAX)) as u16
}

fn terminal_limit_reached(terminal_count: usize) -> bool {
    terminal_count >= MAX_TERMINALS_PER_WORKSPACE
}

fn minimum_layout_extent(node: &LayoutNode, measured_axis: SplitAxis) -> f32 {
    match node {
        LayoutNode::Empty => 0.0,
        LayoutNode::Pane { .. } => match measured_axis {
            SplitAxis::Columns => MIN_PANE_WIDTH,
            SplitAxis::Rows => MIN_PANE_HEIGHT,
        },
        LayoutNode::Split {
            axis,
            first,
            second,
            ..
        } if *axis == measured_axis => {
            minimum_layout_extent(first, measured_axis)
                + DIVIDER_SIZE
                + minimum_layout_extent(second, measured_axis)
        }
        LayoutNode::Split { first, second, .. } => minimum_layout_extent(first, measured_axis)
            .max(minimum_layout_extent(second, measured_axis)),
    }
}

fn internal_divider_extent(node: &LayoutNode, measured_axis: SplitAxis) -> f32 {
    match node {
        LayoutNode::Empty | LayoutNode::Pane { .. } => 0.0,
        LayoutNode::Split {
            axis,
            first,
            second,
            ..
        } if *axis == measured_axis => {
            internal_divider_extent(first, measured_axis)
                + DIVIDER_SIZE
                + internal_divider_extent(second, measured_axis)
        }
        LayoutNode::Split { first, second, .. } => internal_divider_extent(first, measured_axis)
            .max(internal_divider_extent(second, measured_axis)),
    }
}

fn allocation_ratio_for_layout(
    layout_ratio: f32,
    available: f32,
    first_dividers: f32,
    second_dividers: f32,
) -> f32 {
    if available <= 0.0 {
        return 0.5;
    }
    let content_extent = (available - first_dividers - second_dividers).max(0.0);
    (first_dividers + content_extent * layout_ratio) / available
}

fn layout_ratio_for_allocation(
    allocation_ratio: f32,
    available: f32,
    first_dividers: f32,
    second_dividers: f32,
) -> f32 {
    let content_extent = available - first_dividers - second_dividers;
    if content_extent <= 0.0 {
        return allocation_ratio.clamp(0.1, 0.9);
    }
    ((allocation_ratio * available - first_dividers) / content_extent).clamp(0.1, 0.9)
}

fn constrained_split_ratio(
    ratio: f32,
    available: f32,
    first_minimum: f32,
    second_minimum: f32,
) -> f32 {
    if available <= 0.0 {
        return 0.5;
    }
    let desired = ratio.clamp(0.1, 0.9);
    if first_minimum + second_minimum > available {
        // There is not enough room for every pane's preferred minimum. Keeping the requested
        // proportion makes managed 2/3/4/5/6 grids scale evenly instead of allowing nested
        // minimum clamps to make the last cells progressively narrower.
        return desired;
    }

    let minimum_ratio = (first_minimum / available).clamp(0.1, 0.9);
    let maximum_ratio = (1.0 - second_minimum / available).clamp(0.1, 0.9);
    desired.clamp(minimum_ratio, maximum_ratio)
}

fn snap_to_pixel(value: f32, pixels_per_point: f32) -> f32 {
    (value * pixels_per_point).round() / pixels_per_point
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn scroll_lines(delta: f32) -> usize {
    // abs() guarantees a non-negative value and UI wheel deltas are bounded in practice.
    (delta.abs() / 12.0).ceil().clamp(1.0, 100.0) as usize
}

fn compact_path(path: &Path) -> String {
    let text = path.display().to_string();
    if text.chars().count() <= 34 {
        return text;
    }
    let tail: String = text
        .chars()
        .rev()
        .take(29)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("…{tail}")
}

fn compact_header_path(path: &Path, max_chars: usize) -> String {
    let mut text = path.display().to_string();
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let profile = PathBuf::from(profile).display().to_string();
        if text
            .get(..profile.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&profile))
        {
            text.replace_range(..profile.len(), "~");
        }
    }
    if text.chars().count() <= max_chars {
        return text;
    }
    if max_chars <= 4 {
        return text.chars().take(max_chars).collect();
    }
    let tail: String = text
        .chars()
        .rev()
        .take(max_chars - 2)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("…\\{tail}")
}

fn read_git_status(cwd: &Path) -> Option<GitStatus> {
    let status = git_output(
        cwd,
        &[
            "status",
            "--porcelain=v1",
            "--branch",
            "--untracked-files=normal",
        ],
    )?;
    let numstat = git_output(cwd, &["diff", "--numstat", "HEAD"]).unwrap_or_default();
    let detached = git_output(cwd, &["rev-parse", "--short", "HEAD"]);
    parse_git_status(&status, &numstat, detached.as_deref())
}

fn git_output(cwd: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(arguments)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn parse_git_status(status: &str, numstat: &str, detached_head: Option<&str>) -> Option<GitStatus> {
    let mut lines = status.lines();
    let branch_line = lines.next()?.strip_prefix("## ")?;
    let branch = if branch_line.starts_with("HEAD ") {
        detached_head.unwrap_or("detached").trim().to_owned()
    } else {
        branch_line
            .strip_prefix("No commits yet on ")
            .unwrap_or(branch_line)
            .split("...")
            .next()
            .unwrap_or(branch_line)
            .split(" [")
            .next()
            .unwrap_or(branch_line)
            .trim()
            .to_owned()
    };
    let changed_files = lines.count();
    let (additions, deletions) = numstat.lines().fold((0_usize, 0_usize), |totals, line| {
        let mut fields = line.split('\t');
        let added = fields
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let removed = fields
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        (totals.0 + added, totals.1 + removed)
    });
    Some(GitStatus {
        branch,
        changed_files,
        additions,
        deletions,
    })
}

fn compact_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut compact: String = text.chars().take(max_chars.saturating_sub(3)).collect();
    compact.push_str("...");
    compact
}

fn terminal_color(color: vt100::Color, background: bool) -> Color32 {
    match color {
        vt100::Color::Default if background => terminal_background(),
        vt100::Color::Default => Color32::from_rgb(214, 218, 211),
        vt100::Color::Rgb(red, green, blue) => Color32::from_rgb(red, green, blue),
        vt100::Color::Idx(index) => indexed_color(index),
    }
}

fn indexed_color(index: u8) -> Color32 {
    const ANSI: [(u8, u8, u8); 16] = [
        (28, 30, 28),
        (205, 73, 69),
        (82, 171, 103),
        (218, 177, 83),
        (79, 135, 218),
        (180, 99, 193),
        (70, 174, 182),
        (211, 214, 207),
        (101, 105, 99),
        (239, 102, 98),
        (112, 207, 132),
        (245, 207, 112),
        (112, 163, 236),
        (211, 137, 224),
        (99, 207, 215),
        (249, 250, 247),
    ];
    match index {
        0..=15 => {
            let (red, green, blue) = ANSI[index as usize];
            Color32::from_rgb(red, green, blue)
        }
        16..=231 => {
            let value = index - 16;
            let red = value / 36;
            let green = (value % 36) / 6;
            let blue = value % 6;
            let component = |part: u8| if part == 0 { 0 } else { 55 + part * 40 };
            Color32::from_rgb(component(red), component(green), component(blue))
        }
        _ => {
            let gray = 8 + (index - 232) * 10;
            Color32::from_gray(gray)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_terminal_selection_is_normalized() {
        let selection = TerminalSelection {
            start: CellPosition { row: 2, column: 4 },
            end: CellPosition { row: 1, column: 3 },
        };
        let (start, end) = ordered_selection(selection, 80);
        assert_eq!((start.row, start.column), (1, 3));
        assert_eq!((end.row, end.column), (2, 4));
    }

    #[test]
    fn terminal_grid_uses_only_complete_cells() {
        assert_eq!(cells_for_pixels(803.0, 8.0), 100);
        assert_eq!(cells_for_pixels(359.0, 18.0), 19);
        assert_eq!(cells_for_pixels(1.0, 18.0), 2);
    }

    #[test]
    fn terminal_limit_popup_starts_at_six_terminals() {
        assert!(!terminal_limit_reached(5));
        assert!(terminal_limit_reached(6));
        assert!(terminal_limit_reached(7));
    }

    #[test]
    fn terminal_block_markers_become_divider_rows() {
        let mut parser = vt100::Parser::new(6, 80, 100);
        parser.process(
            format!("output\r\n\x1b[8m{TERMINAL_DIVIDER_MARKER}\x1b[0m\r\nPS> ").as_bytes(),
        );

        assert_eq!(terminal_divider_rows(parser.screen()), vec![1]);
        assert!(!is_terminal_divider_row(parser.screen(), 0));
        assert!(!is_terminal_divider_row(parser.screen(), 2));
    }

    #[test]
    fn compact_text_preserves_short_names_and_truncates_long_ones() {
        assert_eq!(compact_text("workspace", 12), "workspace");
        assert_eq!(compact_text("a very long workspace", 12), "a very lo...");
    }

    #[test]
    fn git_status_parses_branch_and_worktree_totals() {
        let status = "## feature/live-git...origin/feature/live-git\n M src/main.rs\nA  icon.svg";
        let numstat = "97\t4\tsrc/main.rs\n12\t0\ticon.svg";
        let git = parse_git_status(status, numstat, None).unwrap();

        assert_eq!(git.branch, "feature/live-git");
        assert_eq!(git.changed_files, 2);
        assert_eq!((git.additions, git.deletions), (109, 4));
    }

    #[test]
    fn git_status_handles_clean_detached_head() {
        let git = parse_git_status("## HEAD (no branch)", "", Some("a1b2c3d")).unwrap();

        assert_eq!(git.branch, "a1b2c3d");
        assert_eq!(git.changed_files, 0);
        assert_eq!((git.additions, git.deletions), (0, 0));
    }

    #[test]
    fn command_palette_search_is_case_insensitive_and_trimmed() {
        assert!(palette_matches("New Workspace", " workspace "));
        assert!(palette_matches("Split Pane Right", "SPLIT"));
        assert!(!palette_matches("Close Workspace", "rename"));
        assert!(palette_matches("Close Workspace", ""));
    }

    #[test]
    fn workspace_identity_colors_are_stable_and_use_vercel_palette() {
        let workspace_id = ade_core::WorkspaceId::new();
        let colors = workspace_identity_colors(workspace_id);
        assert_eq!(colors, workspace_identity_colors(workspace_id));
        assert!(
            [
                Color32::from_rgb(0x00, 0x2f, 0x62),
                Color32::from_rgb(0x5d, 0x0e, 0x17),
                Color32::from_rgb(0x50, 0x28, 0x00),
                Color32::from_rgb(0x00, 0x3a, 0x0e),
                Color32::from_rgb(0x00, 0x3d, 0x34),
                Color32::from_rgb(0x47, 0x18, 0x5e),
                Color32::from_rgb(0x57, 0x10, 0x32),
            ]
            .contains(&colors.0)
        );
    }

    #[test]
    fn split_ratio_keeps_both_panes_usable() {
        let left = constrained_split_ratio(0.05, 900.0, 220.0, 220.0);
        let right = constrained_split_ratio(0.95, 900.0, 220.0, 220.0);
        let crowded = constrained_split_ratio(0.2, 300.0, 220.0, 220.0);

        assert!((left - 220.0 / 900.0).abs() < f32::EPSILON);
        assert!((right - 680.0 / 900.0).abs() < f32::EPSILON);
        assert!((crowded - 0.2).abs() < f32::EPSILON);
    }

    #[test]
    fn nested_grid_minimums_account_for_every_terminal() {
        let panes: Vec<_> = (0..3).map(|_| PaneId::new()).collect();
        let layout = ade_core::managed_terminal_layout(&panes);

        assert!(
            (minimum_layout_extent(&layout, SplitAxis::Columns)
                - (3.0 * MIN_PANE_WIDTH + 2.0 * DIVIDER_SIZE))
                .abs()
                < f32::EPSILON
        );
        assert!(
            (minimum_layout_extent(&layout, SplitAxis::Rows) - MIN_PANE_HEIGHT).abs()
                < f32::EPSILON
        );
        assert!(
            (internal_divider_extent(&layout, SplitAxis::Columns) - 2.0 * DIVIDER_SIZE).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn crowded_three_column_grid_preserves_equal_proportions() {
        let available = 594.0;
        let root = allocation_ratio_for_layout(1.0 / 3.0, available, 0.0, DIVIDER_SIZE);
        let root = constrained_split_ratio(
            root,
            available,
            MIN_PANE_WIDTH,
            2.0 * MIN_PANE_WIDTH + DIVIDER_SIZE,
        );
        let first_width = available * root;
        let remaining = available * (1.0 - root) - DIVIDER_SIZE;
        let nested = allocation_ratio_for_layout(0.5, remaining, 0.0, 0.0);
        let nested = constrained_split_ratio(nested, remaining, MIN_PANE_WIDTH, MIN_PANE_WIDTH);
        let second_width = remaining * nested;
        let third_width = remaining * (1.0 - nested);

        assert!((nested - 0.5).abs() < f32::EPSILON);
        assert!((first_width - second_width).abs() < f32::EPSILON);
        assert!((second_width - third_width).abs() < f32::EPSILON);
    }

    #[test]
    fn managed_grids_stay_even_at_wide_and_compact_window_sizes() {
        let panes: Vec<_> = (0..MAX_TERMINALS_PER_WORKSPACE)
            .map(|_| PaneId::new())
            .collect();

        for count in [2, 3, 4, 6] {
            let layout = ade_core::managed_terminal_layout(&panes[..count]);
            for (width, height) in [(1_440.0, 900.0), (640.0, 360.0)] {
                let mut sizes = Vec::new();
                collect_pane_sizes(&layout, width, height, &mut sizes);
                let (expected_width, expected_height) = sizes[0];

                assert_eq!(sizes.len(), count);
                assert!(sizes.iter().all(|(pane_width, pane_height)| {
                    (pane_width - expected_width).abs() < 0.01
                        && (pane_height - expected_height).abs() < 0.01
                }));
            }
        }
    }

    fn collect_pane_sizes(node: &LayoutNode, width: f32, height: f32, sizes: &mut Vec<(f32, f32)>) {
        let LayoutNode::Split {
            axis,
            ratio,
            first,
            second,
        } = node
        else {
            if matches!(node, LayoutNode::Pane { .. }) {
                sizes.push((width, height));
            }
            return;
        };
        let extent = match axis {
            SplitAxis::Columns => width,
            SplitAxis::Rows => height,
        };
        let available = extent - DIVIDER_SIZE;
        let allocation = allocation_ratio_for_layout(
            *ratio,
            available,
            internal_divider_extent(first, *axis),
            internal_divider_extent(second, *axis),
        );
        let allocation = constrained_split_ratio(
            allocation,
            available,
            minimum_layout_extent(first, *axis),
            minimum_layout_extent(second, *axis),
        );
        let first_extent = available * allocation;
        let second_extent = available * (1.0 - allocation);

        match axis {
            SplitAxis::Columns => {
                collect_pane_sizes(first, first_extent, height, sizes);
                collect_pane_sizes(second, second_extent, height, sizes);
            }
            SplitAxis::Rows => {
                collect_pane_sizes(first, width, first_extent, sizes);
                collect_pane_sizes(second, width, second_extent, sizes);
            }
        }
    }

    #[test]
    fn pointer_mapping_accounts_for_bottom_anchored_grid() {
        let content_rect =
            egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(210.0, 210.0));
        let grid_origin = egui::pos2(10.0, 150.0);
        let position = cell_at_pointer(
            egui::pos2(35.0, 175.0),
            content_rect,
            grid_origin,
            3,
            20,
            10.0,
            20.0,
        )
        .unwrap();

        assert_eq!((position.row, position.column), (1, 2));
        assert!(
            cell_at_pointer(
                egui::pos2(35.0, 120.0),
                content_rect,
                grid_origin,
                3,
                20,
                10.0,
                20.0,
            )
            .is_none()
        );
    }
}
