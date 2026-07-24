#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Pipes::PeekNamedPipe;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_INSERT, VK_V};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

const SCROLLBACK_LINES: usize = 10_000;
const DIVIDER_SIZE: f32 = 6.0;
const MIN_PANE_WIDTH: f32 = 220.0;
const MIN_PANE_HEIGHT: f32 = 120.0;
const GIT_REFRESH_INTERVAL: Duration = Duration::from_millis(1500);
const TERMINAL_DIVIDER_MARKER: &str = "__ADE_BLOCK_DIVIDER__";
const TERMINAL_DIVIDER_OFFSET: f32 = 7.0;
const TERMINAL_REVEAL_DURATION: Duration = Duration::from_millis(160);
const TERMINAL_REVEAL_OFFSET: f32 = 4.0;
const TERMINAL_CLOSE_DURATION: Duration = Duration::from_millis(220);
const TERMINAL_CURSOR_STEADY_DURATION: Duration = Duration::from_millis(520);
const TERMINAL_CURSOR_BLINK_PERIOD: Duration = Duration::from_millis(1_100);
const TERMINAL_CURSOR_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const SYNCHRONIZED_OUTPUT_TIMEOUT: Duration = Duration::from_millis(150);
const SYNCHRONIZED_OUTPUT_LIMIT: usize = 1024 * 1024;
const SYNCHRONIZED_OUTPUT_BEGIN: &[u8] = b"\x1b[?2026h";
const SYNCHRONIZED_OUTPUT_END: &[u8] = b"\x1b[?2026l";
const RECENT_COMMAND_OSC_PREFIX: &[u8] = b"\x1b]6973;";
const RECENT_COMMAND_LIMIT: usize = 4096;
const SIDEBAR_BREAKPOINT: f32 = 600.0;
const SIDEBAR_WIDTH: f32 = 256.0;
const TABLET_SIDEBAR_WIDTH: f32 = 224.0;
const SIDEBAR_ROW_HEIGHT: f32 = 40.0;
const COLLAPSED_SIDEBAR_WIDTH: f32 = 56.0;
const WINDOW_TITLE_BAR_HEIGHT: f32 = 36.0;
const SIDEBAR_TRIGGER_WIDTH: f32 = 16.0;
const SIDEBAR_OPEN_DELAY: Duration = Duration::from_millis(180);
const SIDEBAR_CLOSE_DELAY: Duration = Duration::from_millis(450);
const UPDATE_IDLE_DURATION: Duration = Duration::from_mins(5);
const CODEX_USAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(20);
const CODEX_USAGE_HOVER_BRIDGE: Duration = Duration::from_millis(360);
const CHATGPT_LOGO_SVG: &[u8] = br##"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path fill="#fff" d="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z"/></svg>"##;
const OPENCODE_LOGO_SVG: &[u8] = br##"<svg viewBox="0 0 300 300" fill="none" xmlns="http://www.w3.org/2000/svg"><g transform="translate(30 0)"><path d="M180 240H60V120H180V240Z" fill="#4B4646"/><path d="M180 60H60V240H180V60ZM240 300H0V0H240V300Z" fill="#F1ECEC"/></g></svg>"##;
const SETTINGS_GEAR_SVG: &[u8] = br##"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.38a2 2 0 0 0-.73-2.73l-.15-.09a2 2 0 0 1-1-1.74v-.51a2 2 0 0 1 1-1.72l.15-.1a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2Z"/><circle cx="12" cy="12" r="3"/></svg>"##;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const RELEASE_REPOSITORY_OWNER: &str = "GitNimay";
const RELEASE_REPOSITORY_NAME: &str = "ADE-agentic-coding-environment";
// Keep the release asset name different from the name self_update extracts to. For a plain
// executable, identical names make the library open its download as the extraction destination
// and truncate it before replacement.
const RELEASE_ASSET_NAME: &str = "windows-x64-termy.exe";
const UI_SETTINGS_STORAGE_KEY: &str = "termy-ui-settings";

enum UpdateEvent {
    CheckComplete(Option<String>),
    Installed(String),
    Failed(String),
}

#[derive(Clone)]
enum AppUpdateState {
    Idle,
    Checking,
    Available {
        version: String,
        error: Option<String>,
    },
    Installing {
        version: String,
        restart_after: bool,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct PersistedUiSettings {
    auto_expand_sidebar: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CodexUsageSnapshot {
    plan_type: Option<String>,
    primary: Option<CodexUsageWindow>,
    secondary: Option<CodexUsageWindow>,
    credits_balance: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CodexUsageWindow {
    used_percent: u8,
    window_duration_mins: Option<i64>,
    resets_at: Option<i64>,
}

struct CodexUsageMonitor {
    results: Receiver<Result<CodexUsageSnapshot, String>>,
    stop_sender: Sender<()>,
    snapshot: Option<CodexUsageSnapshot>,
    unavailable: bool,
    panel_hovered: bool,
    keep_open_until: Option<Instant>,
}

impl CodexUsageMonitor {
    fn new(context: &egui::Context) -> Self {
        let (result_sender, results) = unbounded();
        let (stop_sender, stop_results) = unbounded();
        let repaint_context = context.clone();
        let worker_started = match thread::Builder::new()
            .name("termy-codex-usage".to_owned())
            .spawn(move || {
                loop {
                    let result = poll_codex_usage();
                    if result_sender.send(result).is_err() {
                        break;
                    }
                    repaint_context.request_repaint();
                    if stop_results
                        .recv_timeout(CODEX_USAGE_REFRESH_INTERVAL)
                        .is_ok()
                    {
                        break;
                    }
                }
            }) {
            Ok(_) => true,
            Err(error) => {
                diagnostic_log(&format!("could not start Codex usage monitor: {error}"));
                false
            }
        };
        Self {
            results,
            stop_sender,
            snapshot: None,
            unavailable: !worker_started,
            panel_hovered: false,
            keep_open_until: None,
        }
    }

    fn drain(&mut self) {
        while let Ok(result) = self.results.try_recv() {
            match result {
                Ok(snapshot) => {
                    self.snapshot = Some(snapshot);
                    self.unavailable = false;
                }
                Err(error) => {
                    if !self.unavailable {
                        diagnostic_log(&format!("Codex usage is unavailable: {error}"));
                    }
                    self.unavailable = true;
                }
            }
        }
    }

    fn show(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        self.drain();
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(34.0, WINDOW_TITLE_BAR_HEIGHT),
            Sense::focusable_noninteractive(),
        );
        let minimum_remaining = self
            .snapshot
            .as_ref()
            .and_then(minimum_codex_remaining_percent);
        let label = minimum_remaining.map_or_else(
            || "Codex usage".to_owned(),
            |remaining| format!("Codex usage, {remaining}% remaining"),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
        });
        let now = Instant::now();
        if response.hovered() || response.has_focus() || self.panel_hovered {
            self.keep_open_until = Some(now + CODEX_USAGE_HOVER_BRIDGE);
        }
        let open = self.keep_open_until.is_some_and(|deadline| deadline > now);
        let reveal = context.animate_bool_with_time_and_easing(
            egui::Id::new("codex-usage-reveal"),
            open,
            0.16,
            egui::emath::easing::cubic_out,
        );

        if response.hovered() || response.has_focus() {
            ui.painter()
                .rect_filled(rect.shrink2(Vec2::new(3.0, 4.0)), 7.0, surface_hover());
        }
        paint_codex_mark(
            ui,
            rect,
            if self.unavailable && self.snapshot.is_none() {
                text_disabled()
            } else {
                text_primary()
            },
            1.0,
        );
        let status_color = match minimum_remaining {
            Some(remaining) => codex_usage_color(remaining),
            None => text_disabled(),
        };
        ui.painter()
            .circle_filled(rect.center() + Vec2::new(7.0, 7.0), 1.75, status_color);

        if reveal > 0.01 {
            let width = (context.content_rect().width() - 48.0).clamp(240.0, 360.0);
            let position = egui::pos2(
                rect.right() - width,
                rect.bottom() - 2.0 - 4.0 * (1.0 - reveal),
            );
            let area = egui::Area::new(egui::Id::new("codex-usage-panel"))
                .order(egui::Order::Foreground)
                .fixed_pos(position)
                .show(context, |ui| {
                    ui.set_width(width);
                    egui::Frame::NONE
                        .fill(Color32::from_rgb(10, 10, 10))
                        .stroke(Stroke::new(1.0, border()))
                        .corner_radius(12.0)
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 8],
                            blur: 24,
                            spread: 0,
                            color: Color32::from_black_alpha(100).gamma_multiply(reveal),
                        })
                        .inner_margin(egui::Margin::same(16))
                        .show(ui, |ui| {
                            ui.set_opacity(reveal);
                            show_codex_usage_panel(ui, self.snapshot.as_ref(), self.unavailable);
                        });
                });
            self.panel_hovered = context
                .pointer_hover_pos()
                .is_some_and(|pointer| area.response.rect.contains(pointer));
            context.request_repaint_after(Duration::from_millis(16));
        } else {
            self.panel_hovered = false;
            self.keep_open_until = None;
        }
        response.on_hover_cursor(egui::CursorIcon::Default);
    }
}

impl Drop for CodexUsageMonitor {
    fn drop(&mut self) {
        let _ = self.stop_sender.send(());
    }
}

fn poll_codex_usage() -> Result<CodexUsageSnapshot, String> {
    let executable = find_codex_executable()
        .ok_or_else(|| "the Codex CLI executable was not found".to_owned())?;
    let mut child = Command::new(executable)
        .args(["app-server", "--stdio"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start Codex: {error}"))?;
    let result = (|| {
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| "Codex stdin is unavailable".to_owned())?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| "Codex stdout is unavailable".to_owned())?;
        let mut lines = BufReader::new(output).lines();
        writeln!(
            input,
            "{}",
            serde_json::json!({
                "method": "initialize",
                "id": 0,
                "params": {
                    "clientInfo": {
                        "name": "termy",
                        "title": "Termy",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": { "experimentalApi": true }
                }
            })
        )
        .map_err(|error| error.to_string())?;
        input.flush().map_err(|error| error.to_string())?;
        read_codex_response(&mut lines, 0)?;

        writeln!(
            input,
            "{}",
            serde_json::json!({ "method": "initialized", "params": {} })
        )
        .map_err(|error| error.to_string())?;
        writeln!(
            input,
            "{}",
            serde_json::json!({ "method": "account/rateLimits/read", "id": 1 })
        )
        .map_err(|error| error.to_string())?;
        input.flush().map_err(|error| error.to_string())?;
        let value = read_codex_response(&mut lines, 1)?;
        parse_codex_usage_snapshot(
            value
                .get("result")
                .ok_or_else(|| "Codex returned no usage result".to_owned())?,
        )
        .ok_or_else(|| "Codex returned no rate-limit windows".to_owned())
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn find_codex_executable() -> Option<PathBuf> {
    let app_data = std::env::var_os("APPDATA").map(PathBuf::from);
    if let Some(app_data) = app_data {
        let npm_package = app_data
            .join("npm")
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("node_modules")
            .join("@openai")
            .join("codex-win32-x64")
            .join("vendor")
            .join("x86_64-pc-windows-msvc")
            .join("bin")
            .join("codex.exe");
        if npm_package.is_file() {
            return Some(npm_package);
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            let candidate = directory.join("codex.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn read_codex_response(
    lines: &mut impl Iterator<Item = io::Result<String>>,
    expected_id: i64,
) -> Result<serde_json::Value, String> {
    for line in lines {
        let line = line.map_err(|error| error.to_string())?;
        let value: serde_json::Value =
            serde_json::from_str(&line).map_err(|error| error.to_string())?;
        if value.get("id").and_then(serde_json::Value::as_i64) != Some(expected_id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(format!("Codex app-server error: {error}"));
        }
        return Ok(value);
    }
    Err("Codex app-server disconnected".to_owned())
}

fn parse_codex_usage_snapshot(value: &serde_json::Value) -> Option<CodexUsageSnapshot> {
    let fallback = value.get("rateLimits")?;
    let limits = value
        .get("rateLimitsByLimitId")
        .and_then(serde_json::Value::as_object)
        .and_then(|limits| limits.get("codex").or_else(|| limits.values().next()))
        .unwrap_or(fallback);
    let parse_window = |name: &str| {
        let window = limits.get(name)?;
        Some(CodexUsageWindow {
            used_percent: window
                .get("usedPercent")?
                .as_u64()?
                .min(100)
                .try_into()
                .ok()?,
            window_duration_mins: window
                .get("windowDurationMins")
                .and_then(serde_json::Value::as_i64),
            resets_at: window.get("resetsAt").and_then(serde_json::Value::as_i64),
        })
    };
    let snapshot = CodexUsageSnapshot {
        plan_type: limits
            .get("planType")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        primary: parse_window("primary"),
        secondary: parse_window("secondary"),
        credits_balance: limits
            .pointer("/credits/balance")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    };
    (snapshot.primary.is_some() || snapshot.secondary.is_some()).then_some(snapshot)
}

fn official_build_version() -> Option<&'static str> {
    option_env!("TERMY_BUILD_VERSION")
}

fn current_app_version() -> &'static str {
    official_build_version().unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn configured_updater(
    current_version: &str,
    target_version: Option<&str>,
) -> Result<Box<dyn self_update::update::ReleaseUpdate>, String> {
    let mut updater = self_update::backends::github::Update::configure();
    updater
        .repo_owner(RELEASE_REPOSITORY_OWNER)
        .repo_name(RELEASE_REPOSITORY_NAME)
        .bin_name("termy")
        .identifier(RELEASE_ASSET_NAME)
        .current_version(current_version)
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(false);
    if let Some(version) = target_version {
        updater.target_version_tag(&format!("v{version}"));
    }
    updater.build().map_err(|error| error.to_string())
}

fn check_latest_release(current_version: &str) -> Result<Option<String>, String> {
    let updater = configured_updater(current_version, None)?;
    let release = updater
        .get_latest_release()
        .map_err(|error| error.to_string())?;
    self_update::version::bump_is_greater(current_version, &release.version)
        .map(|newer| newer.then_some(release.version))
        .map_err(|error| error.to_string())
}

fn install_release(current_version: &str, target_version: &str) -> Result<String, String> {
    let status = configured_updater(current_version, Some(target_version))?
        .update()
        .map_err(|error| error.to_string())?;
    Ok(status.version().to_owned())
}

fn deferred_update_delay(last_activity: Instant, now: Instant) -> Option<Duration> {
    let idle = now.saturating_duration_since(last_activity);
    (idle < UPDATE_IDLE_DURATION).then(|| {
        UPDATE_IDLE_DURATION
            .checked_sub(idle)
            .expect("idle duration was checked")
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().any(|argument| argument == "--daemon") {
        ade_daemon::run_daemon()?;
        return Ok(());
    }

    // A stable identity keeps taskbar pins and running windows grouped as the same application.
    unsafe { SetCurrentProcessExplicitAppUserModelID(windows::core::w!("GitNimay.Termy"))? };
    let app_icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/app-icon.png"))?;
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("termy")
            .with_icon(app_icon)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 360.0])
            .with_decorations(false)
            .with_maximized(true),
        ..Default::default()
    };

    eframe::run_native(
        "termy",
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
        if !matches!(event.message, ServerEvent::TerminalOutput { .. }) {
            diagnostic_log(&format!("read event: {}", event_summary(&event.message)));
        }
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

fn clipboard_images_dir() -> Option<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(local_app_data)
            .join("ADE")
            .join("clipboard-images"),
    )
}

fn save_clipboard_image() -> Result<PathBuf, arboard::Error> {
    let mut clipboard = arboard::Clipboard::new()?;
    let image = clipboard.get_image()?;
    let dir = clipboard_images_dir().ok_or(arboard::Error::ConversionFailure)?;
    std::fs::create_dir_all(&dir).map_err(|_| arboard::Error::ConversionFailure)?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let timestamp = now.as_secs();
    let nanos = now.subsec_nanos();
    let filename = format!("clip_{timestamp}_{nanos}.png");
    let path = dir.join(&filename);

    let rgba = image.bytes;
    let img = image::RgbaImage::from_raw(
        image
            .width
            .try_into()
            .map_err(|_| arboard::Error::ConversionFailure)?,
        image
            .height
            .try_into()
            .map_err(|_| arboard::Error::ConversionFailure)?,
        rgba.into_owned(),
    )
    .ok_or(arboard::Error::ConversionFailure)?;
    img.save(&path)
        .map_err(|_| arboard::Error::ConversionFailure)?;

    Ok(path)
}

fn quoted_terminal_path(path: &Path) -> String {
    format!("\"{}\"", path.to_string_lossy())
}

fn paste_shortcut_is_down(modifiers: Modifiers) -> bool {
    // egui-winit consumes image-only paste shortcuts without emitting an event, so inspect the
    // native key state while raw input from that key press is being prepared.
    let v_down = unsafe { GetAsyncKeyState(i32::from(VK_V.0)) < 0 };
    let insert_down = unsafe { GetAsyncKeyState(i32::from(VK_INSERT.0)) < 0 };
    (modifiers.command && v_down) || (modifiers.shift && insert_down)
}

fn cleanup_old_clipboard_images() {
    let Some(dir) = clipboard_images_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return;
    };
    let thirty_days = Duration::from_hours(720);
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(modified_since_epoch) = modified.duration_since(UNIX_EPOCH) else {
            continue;
        };
        let age = now.saturating_sub(modified_since_epoch);
        if age > thirty_days {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[allow(clippy::struct_excessive_bools)]
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
    settings_open: bool,
    settings_section: SettingsSection,
    auto_expand_sidebar: bool,
    terminal_limit_popup: bool,
    close_requested: bool,
    shutdown_requested: bool,
    paste_shortcut_down: bool,
    update_results: Receiver<UpdateEvent>,
    update_sender: Sender<UpdateEvent>,
    update_state: AppUpdateState,
    deferred_update: Option<String>,
    last_user_activity: Instant,
    restart_executable: Option<PathBuf>,
    codex_usage: CodexUsageMonitor,
}

impl AdeApp {
    fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        configure_style(&creation_context.egui_ctx);
        egui_extras::install_image_loaders(&creation_context.egui_ctx);
        let client = DaemonClient::connect(&creation_context.egui_ctx);
        let error_message = client.as_ref().err().map(ToString::to_string);
        let (update_sender, update_results) = unbounded();
        let mut update_state = AppUpdateState::Idle;
        if let Some(version) = official_build_version() {
            update_state = AppUpdateState::Checking;
            let repaint_context = creation_context.egui_ctx.clone();
            let sender = update_sender.clone();
            if let Err(error) = thread::Builder::new()
                .name("termy-auto-update".to_owned())
                .spawn(move || {
                    let event = match check_latest_release(version) {
                        Ok(available) => UpdateEvent::CheckComplete(available),
                        Err(error) => UpdateEvent::Failed(error),
                    };
                    let _ = sender.send(event);
                    repaint_context.request_repaint();
                })
            {
                diagnostic_log(&format!("could not start update check: {error}"));
                update_state = AppUpdateState::Idle;
            }
        }
        let persisted_ui = creation_context
            .storage
            .and_then(|storage| {
                eframe::get_value::<PersistedUiSettings>(storage, UI_SETTINGS_STORAGE_KEY)
            })
            .unwrap_or_default();
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
            settings_open: false,
            settings_section: SettingsSection::General,
            auto_expand_sidebar: persisted_ui.auto_expand_sidebar,
            terminal_limit_popup: false,
            close_requested: false,
            shutdown_requested: false,
            paste_shortcut_down: false,
            update_results,
            update_sender,
            update_state,
            deferred_update: None,
            last_user_activity: Instant::now(),
            restart_executable: std::env::current_exe().ok(),
            codex_usage: CodexUsageMonitor::new(&creation_context.egui_ctx),
        };
        cleanup_old_clipboard_images();
        app.send(ClientRequest::Attach);
        app
    }

    fn send(&mut self, request: ClientRequest) {
        let Some(client) = &self.client else {
            self.error_message = Some("The termy background daemon is not connected".to_owned());
            return;
        };
        if client.send(request).is_err() {
            self.error_message = Some("The termy background daemon disconnected".to_owned());
        }
    }

    fn has_active_sessions(&self) -> bool {
        self.workspaces.iter().any(|ws| {
            ws.panes.values().any(|pane| {
                matches!(
                    pane.status,
                    SessionStatus::Starting | SessionStatus::Running
                )
            })
        })
    }

    fn perform_shutdown(&mut self, ui: &egui::Ui) {
        self.shutdown_requested = true;
        if let Some(client) = &self.client {
            let _ = client.send(ClientRequest::Shutdown);
        }
        ui.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn drain_update_results(&mut self, context: &egui::Context) {
        while let Ok(event) = self.update_results.try_recv() {
            match event {
                UpdateEvent::CheckComplete(Some(version)) => {
                    self.update_state = AppUpdateState::Available {
                        version,
                        error: None,
                    };
                }
                UpdateEvent::CheckComplete(None) => self.update_state = AppUpdateState::Idle,
                UpdateEvent::Installed(version) => {
                    let restart_after = matches!(
                        self.update_state,
                        AppUpdateState::Installing {
                            restart_after: true,
                            ..
                        }
                    );
                    self.deferred_update = None;
                    self.update_state = AppUpdateState::Idle;
                    diagnostic_log(&format!("installed Termy {version}"));
                    if restart_after && let Err(error) = self.restart_after_update(context) {
                        diagnostic_log(&format!("could not restart after update: {error}"));
                        self.update_state = AppUpdateState::Available {
                            version,
                            error: Some(
                                "Update installed, but Termy could not restart.".to_owned(),
                            ),
                        };
                    }
                }
                UpdateEvent::Failed(error) => {
                    diagnostic_log(&format!("automatic update failed: {error}"));
                    self.update_state = match &self.update_state {
                        AppUpdateState::Installing { version, .. } => AppUpdateState::Available {
                            version: version.clone(),
                            error: Some(
                                "Could not install the update. Try again later.".to_owned(),
                            ),
                        },
                        _ => AppUpdateState::Idle,
                    };
                }
            }
        }
    }

    fn note_user_activity(&mut self, context: &egui::Context) {
        if context.input(|input| !input.events.is_empty() || input.pointer.any_down()) {
            self.last_user_activity = Instant::now();
        }
    }

    fn start_deferred_update_if_idle(&mut self, context: &egui::Context) {
        if !matches!(self.update_state, AppUpdateState::Idle) {
            return;
        }
        let Some(version) = self.deferred_update.clone() else {
            return;
        };
        if let Some(delay) = deferred_update_delay(self.last_user_activity, Instant::now()) {
            context.request_repaint_after(delay);
        } else {
            self.start_update(version, false, context);
        }
    }

    fn start_update(&mut self, version: String, restart_after: bool, context: &egui::Context) {
        let Some(current_version) = official_build_version() else {
            return;
        };
        self.update_state = AppUpdateState::Installing {
            version: version.clone(),
            restart_after,
        };
        self.deferred_update = None;
        let sender = self.update_sender.clone();
        let repaint_context = context.clone();
        let install_version = version.clone();
        if let Err(error) = thread::Builder::new()
            .name("termy-update-install".to_owned())
            .spawn(move || {
                let event = install_release(current_version, &install_version)
                    .map_or_else(UpdateEvent::Failed, UpdateEvent::Installed);
                let _ = sender.send(event);
                repaint_context.request_repaint();
            })
        {
            self.update_state = AppUpdateState::Available {
                version,
                error: Some("Could not start the update. Try again later.".to_owned()),
            };
            diagnostic_log(&format!("could not start update installer: {error}"));
        }
    }

    fn restart_after_update(&mut self, context: &egui::Context) -> io::Result<()> {
        let executable = self
            .restart_executable
            .as_ref()
            .ok_or_else(|| io::Error::other("current executable path is unavailable"))?;
        Command::new(executable)
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .spawn()?;
        // Close only this UI process. The daemon keeps terminal sessions alive for the new window.
        self.shutdown_requested = true;
        context.send_viewport_cmd(egui::ViewportCommand::Close);
        Ok(())
    }

    fn show_update_notice(&mut self, context: &egui::Context) {
        let state = self.update_state.clone();
        let (version, error, installing) = match state {
            AppUpdateState::Available { version, error } => (version, error, false),
            AppUpdateState::Installing { version, .. } => (version, None, true),
            AppUpdateState::Idle | AppUpdateState::Checking => return,
        };
        let width = (context.content_rect().width() - 32.0).clamp(300.0, 390.0);
        let mut action = None;
        egui::Area::new(egui::Id::new("update-notice"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -16.0))
            .show(context, |ui| {
                ui.set_width(width);
                egui::Frame::NONE
                    .fill(Color32::from_rgb(10, 10, 10))
                    .stroke(Stroke::new(1.0, border()))
                    .corner_radius(10.0)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: Color32::from_black_alpha(190),
                    })
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(width - 34.0);
                        ui.horizontal(|ui| {
                            paint_update_icon(ui, installing);
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(if installing {
                                            "Installing update"
                                        } else {
                                            "Update available"
                                        })
                                        .size(14.0)
                                        .strong()
                                        .color(text_primary()),
                                    );
                                    update_version_badge(ui, &version);
                                });
                                ui.add_space(2.0);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(if installing {
                                            "Termy will be ready in a moment. Your terminals stay running."
                                        } else {
                                            "Restart now to use the latest version, or install it quietly when you're idle."
                                        })
                                        .size(12.5)
                                        .color(text_secondary()),
                                    )
                                    .wrap(),
                                );
                            });
                        });
                        if let Some(error) = error {
                            ui.add_space(10.0);
                            ui.label(RichText::new(error).size(12.0).color(Color32::from_rgb(238, 91, 91)));
                        }
                        if !installing {
                            ui.add_space(14.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if update_notice_button(ui, "Update and restart", true).clicked() {
                                        action = Some(true);
                                    }
                                    if update_notice_button(ui, "Later", false).clicked() {
                                        action = Some(false);
                                    }
                                },
                            );
                        }
                    });
            });

        match action {
            Some(true) => self.start_update(version, true, context),
            Some(false) => {
                self.deferred_update = Some(version);
                self.last_user_activity = Instant::now();
                self.update_state = AppUpdateState::Idle;
                context.request_repaint_after(UPDATE_IDLE_DURATION);
            }
            None => {}
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

    #[allow(clippy::too_many_lines)]
    fn handle_shortcuts(&mut self, context: &egui::Context) {
        if context.input_mut(|input| {
            input.consume_shortcut(&shortcut(Key::P))
                || input.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::K))
        }) {
            self.palette_open = true;
            self.palette_query.clear();
            self.palette_selection = 0;
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
            input.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::CTRL | Modifiers::SHIFT,
                Key::ArrowRight,
            ))
        }) && !self.workspaces.is_empty()
        {
            self.focus_terminal_direction(SplitDirection::Right);
        }
        if context.input_mut(|input| {
            input.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::CTRL | Modifiers::SHIFT,
                Key::ArrowLeft,
            ))
        }) && !self.workspaces.is_empty()
        {
            self.focus_terminal_direction(SplitDirection::Left);
        }
        if context.input_mut(|input| {
            input.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::CTRL | Modifiers::SHIFT,
                Key::ArrowDown,
            ))
        }) && !self.workspaces.is_empty()
        {
            self.focus_terminal_direction(SplitDirection::Down);
        }
        if context.input_mut(|input| {
            input.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::CTRL | Modifiers::SHIFT,
                Key::ArrowUp,
            ))
        }) && !self.workspaces.is_empty()
        {
            self.focus_terminal_direction(SplitDirection::Up);
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

    fn focus_terminal_direction(&mut self, direction: SplitDirection) {
        let Some(workspace) = self.workspaces.get_mut(self.active_workspace) else {
            return;
        };
        let Some(current) = workspace.model.active_pane_id else {
            return;
        };
        if let Some(adjacent) = workspace.model.layout.find_adjacent(current, direction) {
            workspace.model.active_pane_id = Some(adjacent);
            self.send(ClientRequest::FocusPane { pane_id: adjacent });
        }
    }

    #[allow(clippy::too_many_lines)]
    fn sidebar(&mut self, root_ui: &mut egui::Ui, context: &egui::Context) {
        if root_ui.available_width() <= SIDEBAR_BREAKPOINT {
            self.compact_workspace_bar(root_ui, context);
            return;
        }

        let mut action = None;
        let mut create_workspace = false;
        let mut open_settings = false;
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
                    let compact_result = compact_sidebar_rail(
                        ui,
                        &self.workspaces,
                        self.active_workspace,
                        self.settings_open,
                    );
                    action = compact_result.action;
                    context_menu_open = compact_result.context_menu_open;
                    create_workspace = compact_result.create_workspace;
                    open_settings = compact_result.open_settings;
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
                        let list_height = (ui.available_height() - 48.0).max(0.0);
                        ui.allocate_ui_with_layout(
                            Vec2::new(ui.available_width(), list_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        for (index, workspace) in self.workspaces.iter().enumerate()
                                        {
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
                            },
                        );
                        ui.painter().hline(
                            ui.max_rect().x_range(),
                            ui.cursor().top(),
                            Stroke::new(1.0, border()),
                        );
                        ui.add_space(7.0);
                        if sidebar_settings_button(ui, self.settings_open)
                            .on_hover_text("Settings")
                            .clicked()
                        {
                            open_settings = true;
                        }
                    });
            });

        let pointer = context.input(|input| input.pointer.hover_pos());
        let edge_hovered = pointer.is_some_and(|position| {
            position.x <= panel.response.rect.left() + SIDEBAR_TRIGGER_WIDTH
                && panel.response.rect.y_range().contains(position.y)
        });
        let panel_hovered = pointer.is_some_and(|position| panel.response.rect.contains(position));
        self.update_sidebar_hover(panel_hovered || edge_hovered, context_menu_open, context);

        if create_workspace {
            self.request_new_workspace(context);
        }
        if open_settings {
            self.settings_open = true;
        }
        self.handle_workspace_action(action);
    }

    fn update_sidebar_hover(
        &mut self,
        hovered: bool,
        context_menu_open: bool,
        context: &egui::Context,
    ) {
        let now = Instant::now();
        if !self.auto_expand_sidebar {
            self.sidebar_open = false;
            self.sidebar_hover_started = None;
            self.sidebar_left_at = None;
            return;
        }
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
                let elapsed = now.duration_since(*left_at);
                if elapsed >= SIDEBAR_CLOSE_DELAY {
                    self.sidebar_open = false;
                    self.sidebar_left_at = None;
                } else {
                    context.request_repaint_after(SIDEBAR_CLOSE_DELAY.saturating_sub(elapsed));
                }
            }
        } else {
            self.sidebar_left_at = None;
            if hovered {
                let hover_started = self.sidebar_hover_started.get_or_insert(now);
                let elapsed = now.duration_since(*hover_started);
                if elapsed >= SIDEBAR_OPEN_DELAY {
                    self.sidebar_open = true;
                    self.sidebar_hover_started = None;
                } else {
                    context.request_repaint_after(SIDEBAR_OPEN_DELAY.saturating_sub(elapsed));
                }
            } else {
                self.sidebar_hover_started = None;
            }
        }
    }

    fn compact_workspace_bar(&mut self, root_ui: &mut egui::Ui, context: &egui::Context) {
        let mut action = None;
        let mut create_workspace = false;
        let mut open_settings = false;
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
                        if topbar_settings_button(ui, self.settings_open)
                            .on_hover_text("Settings")
                            .clicked()
                        {
                            open_settings = true;
                        }
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
        if open_settings {
            self.settings_open = true;
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
    fn settings_page(&mut self, context: &egui::Context) {
        if !self.settings_open {
            return;
        }
        if context.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            self.settings_open = false;
            return;
        }

        let content_rect = context.content_rect();
        let panel_width = (content_rect.width() - 56.0).clamp(320.0, 920.0);
        let panel_height = (content_rect.height() - 64.0).clamp(300.0, 640.0);
        let modal_frame = egui::Frame::NONE
            .fill(vercel_bg())
            .stroke(Stroke::new(1.0, vercel_border()))
            .corner_radius(8.0)
            .shadow(egui::epaint::Shadow {
                offset: [0, 16],
                blur: 40,
                spread: 0,
                color: Color32::from_black_alpha(190),
            });

        let response = egui::Modal::new(egui::Id::new("settings-page-modal"))
            .backdrop_color(Color32::from_black_alpha(162))
            .frame(modal_frame)
            .show(context, |ui| {
                ui.set_width(panel_width);
                ui.set_height(panel_height);
                ui.set_min_size(Vec2::new(panel_width, panel_height));
                let header_height = 58.0;
                let (header_rect, _) =
                    ui.allocate_exact_size(Vec2::new(panel_width, header_height), Sense::hover());
                ui.painter().text(
                    header_rect.left_center() + Vec2::new(24.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    "Settings",
                    FontId::proportional(20.0),
                    vercel_text_primary(),
                );
                let close_rect = egui::Rect::from_center_size(
                    header_rect.right_center() - Vec2::new(24.0, 0.0),
                    Vec2::splat(28.0),
                );
                let close = ui.interact(
                    close_rect,
                    egui::Id::new("settings-page-close"),
                    Sense::click(),
                );
                close.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        ui.is_enabled(),
                        "Close settings",
                    )
                });
                if close.hovered() || close.has_focus() {
                    ui.painter()
                        .rect_filled(close_rect, 6.0, vercel_surface_hover());
                    ui.painter().rect_stroke(
                        close_rect,
                        6.0,
                        Stroke::new(1.0, vercel_border()),
                        egui::StrokeKind::Inside,
                    );
                }
                paint_close_icon(ui.painter(), close_rect.center(), vercel_text_secondary());
                if close.clicked() {
                    ui.close();
                }

                ui.painter().hline(
                    header_rect.x_range(),
                    header_rect.bottom(),
                    Stroke::new(1.0, vercel_border()),
                );

                let body_height = (panel_height - header_height).max(0.0);
                let (body_rect, _) =
                    ui.allocate_exact_size(Vec2::new(panel_width, body_height), Sense::hover());
                let sidebar_width = 204.0_f32.min(panel_width * 0.34);
                let sidebar_rect = egui::Rect::from_min_max(
                    body_rect.min,
                    egui::pos2(body_rect.left() + sidebar_width, body_rect.bottom()),
                );
                ui.painter().vline(
                    sidebar_rect.right(),
                    body_rect.y_range(),
                    Stroke::new(1.0, vercel_border()),
                );
                let mut sidebar = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(sidebar_rect.shrink2(Vec2::new(18.0, 22.0)))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                for section in SettingsSection::ALL {
                    let response = settings_nav_item(
                        &mut sidebar,
                        section.label(),
                        section == self.settings_section,
                    );
                    if response.clicked() {
                        self.settings_section = section;
                    }
                }
                paint_settings_version_footer(ui, sidebar_rect, current_app_version());
                let content_rect = egui::Rect::from_min_max(
                    egui::pos2(sidebar_rect.right(), body_rect.top()),
                    body_rect.max,
                );
                let mut settings_content = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(content_rect.shrink2(Vec2::new(28.0, 24.0)))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                settings_section_content(
                    &mut settings_content,
                    self.settings_section,
                    &mut self.auto_expand_sidebar,
                );
            });

        if response.should_close() {
            self.settings_open = false;
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
            PaletteCommand::FocusTerminalLeft => {
                self.focus_terminal_direction(SplitDirection::Left);
            }
            PaletteCommand::FocusTerminalRight => {
                self.focus_terminal_direction(SplitDirection::Right);
            }
            PaletteCommand::FocusTerminalUp => {
                self.focus_terminal_direction(SplitDirection::Up);
            }
            PaletteCommand::FocusTerminalDown => {
                self.focus_terminal_direction(SplitDirection::Down);
            }
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

    fn show_close_confirmation(&mut self, context: &egui::Context) {
        if !self.close_requested {
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
        let response = egui::Modal::new(egui::Id::new("close-confirmation-popup"))
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
                            RichText::new("Close Termy?")
                                .font(FontId::proportional(16.0))
                                .strong()
                                .color(text_primary()),
                        );
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(
                                "You have active terminal sessions running. Closing the app will terminate all sessions and their processes.",
                            )
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
                            let close = modal_danger_button(ui, "Close");
                            close.request_focus();
                            if close.clicked()
                                || ui.input(|input| input.key_pressed(Key::Enter))
                            {
                                ui.close();
                                self.perform_shutdown(ui);
                            }
                            let cancel = modal_primary_button(ui, "Cancel");
                            if cancel.clicked()
                                || ui.input(|input| input.key_pressed(Key::Escape))
                            {
                                ui.close();
                            }
                        });
                    });
            });
        if response.should_close() {
            self.close_requested = false;
        }
    }
}

impl eframe::App for AdeApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(
            storage,
            UI_SETTINGS_STORAGE_KEY,
            &PersistedUiSettings {
                auto_expand_sidebar: self.auto_expand_sidebar,
            },
        );
    }

    fn raw_input_hook(&mut self, _context: &egui::Context, input: &mut egui::RawInput) {
        let shortcut_down = input.focused && paste_shortcut_is_down(input.modifiers);
        let has_text_paste = input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Paste(_)));
        let terminal_accepts_input = !self.palette_open
            && self.rename_workspace.is_none()
            && !self.settings_open
            && self
                .workspaces
                .get(self.active_workspace)
                .and_then(|workspace| workspace.model.active_pane_id)
                .is_some();

        if shortcut_down && !self.paste_shortcut_down && !has_text_paste && terminal_accepts_input {
            match save_clipboard_image() {
                Ok(path) => input
                    .events
                    .push(egui::Event::Paste(quoted_terminal_path(&path))),
                Err(arboard::Error::ContentNotAvailable) => {}
                Err(error) => {
                    self.error_message = Some(format!("Could not paste clipboard image: {error}"));
                }
            }
        }
        self.paste_shortcut_down = shortcut_down;
    }

    #[allow(clippy::too_many_lines)]
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        self.note_user_activity(&context);
        self.drain_update_results(&context);
        self.start_deferred_update_if_idle(&context);
        self.drain_daemon_events(&context);
        self.handle_shortcuts(&context);
        self.finish_pane_closes(&context);

        // Intercept close requests (from title bar button, Alt+F4, or OS)
        let viewport_close = ui.input(|input| input.viewport().close_requested());
        if viewport_close && !self.shutdown_requested {
            ui.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.close_requested {
                if self.has_active_sessions() {
                    self.close_requested = true;
                } else {
                    self.perform_shutdown(ui);
                }
            }
        }

        let compact = ui.available_width() <= SIDEBAR_BREAKPOINT;
        if compact {
            window_title_bar(ui, &context, &mut self.codex_usage);
            self.sidebar(ui, &context);
        } else {
            self.sidebar(ui, &context);
            window_title_bar(ui, &context, &mut self.codex_usage);
        }

        let requests = self.client.as_ref().map(|client| client.requests.clone());
        let terminal_input_enabled =
            !self.palette_open && self.rename_workspace.is_none() && !self.settings_open;
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
            egui::Window::new("termy error")
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
        self.show_update_notice(&context);
        self.show_terminal_limit_popup(&context);
        self.show_close_confirmation(&context);
        self.workspace_dialogs(&context);
        self.settings_page(&context);
        self.command_palette(&context);
        // Terminal output and user input request repaints immediately. A slow idle tick is enough
        // for background metadata and avoids rebuilding a full terminal grid 30 times per second.
        context.request_repaint_after(GIT_REFRESH_INTERVAL);
    }
}

#[derive(Clone, Copy)]
enum WindowControl {
    Minimize,
    Maximize,
    Close,
}

fn window_title_bar(
    root_ui: &mut egui::Ui,
    context: &egui::Context,
    codex_usage: &mut CodexUsageMonitor,
) {
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
                codex_usage.show(ui, context);

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

fn minimum_codex_remaining_percent(snapshot: &CodexUsageSnapshot) -> Option<u8> {
    snapshot
        .primary
        .iter()
        .chain(snapshot.secondary.iter())
        .map(|window| 100_u8.saturating_sub(window.used_percent))
        .min()
}

fn codex_usage_color(remaining: u8) -> Color32 {
    match remaining {
        0..=5 => danger(),
        6..=20 => Color32::from_rgb(245, 166, 35),
        _ => text_primary(),
    }
}

fn show_codex_usage_panel(
    ui: &mut egui::Ui,
    snapshot: Option<&CodexUsageSnapshot>,
    unavailable: bool,
) {
    ui.horizontal(|ui| {
        let (mark_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        paint_codex_mark(ui, mark_rect, text_primary(), 1.0);
        ui.label(
            RichText::new("Codex")
                .size(14.0)
                .strong()
                .color(text_primary()),
        );
        if let Some(plan) = snapshot.and_then(|snapshot| snapshot.plan_type.as_deref()) {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::Frame::NONE
                    .fill(surface_hover())
                    .stroke(Stroke::new(1.0, border()))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(7, 4))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(codex_plan_label(plan))
                                .size(12.0)
                                .strong()
                                .color(text_primary()),
                        );
                    });
            });
        }
    });
    ui.add_space(16.0);

    if let Some(snapshot) = snapshot {
        for (shown, window) in [snapshot.primary.as_ref(), snapshot.secondary.as_ref()]
            .into_iter()
            .flatten()
            .enumerate()
        {
            if shown > 0 {
                ui.add_space(16.0);
            }
            show_codex_usage_window(ui, window);
        }
        if let Some(balance) = &snapshot.credits_balance {
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Credits").size(13.0).color(text_secondary()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(codex_credits_label(balance))
                            .size(13.0)
                            .family(FontFamily::Monospace)
                            .strong()
                            .color(text_primary()),
                    );
                });
            });
        }
        show_codex_usage_footer(ui, unavailable);
    } else {
        ui.label(
            RichText::new(if unavailable {
                "Codex usage unavailable"
            } else {
                "Connecting to Codex…"
            })
            .size(14.0)
            .strong()
            .color(text_primary()),
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new(if unavailable {
                "Install or sign in to the Codex CLI to view your limits."
            } else {
                "Reading your current account limits securely."
            })
            .size(13.0)
            .color(text_secondary()),
        );
    }
}

fn show_codex_usage_footer(ui: &mut egui::Ui, unavailable: bool) {
    ui.add_space(15.0);
    let (divider, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 1.0 / ui.ctx().pixels_per_point()),
        Sense::hover(),
    );
    ui.painter().rect_filled(divider, 0.0, border());
    ui.add_space(11.0);
    ui.horizontal(|ui| {
        ui.painter().circle_filled(
            ui.cursor().left_center() + Vec2::new(2.5, 0.0),
            2.5,
            if unavailable {
                Color32::from_rgb(245, 166, 35)
            } else {
                Color32::from_rgb(70, 167, 88)
            },
        );
        ui.add_space(9.0);
        ui.label(
            RichText::new(if unavailable {
                "Reconnecting · showing last update"
            } else {
                "Live · refreshes every 20 seconds"
            })
            .size(12.0)
            .color(text_secondary()),
        );
    });
}

fn show_codex_usage_window(ui: &mut egui::Ui, window: &CodexUsageWindow) {
    let remaining = 100_u8.saturating_sub(window.used_percent);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(codex_window_label(window.window_duration_mins))
                .size(13.0)
                .color(text_secondary()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{remaining}% left"))
                    .size(13.0)
                    .family(FontFamily::Monospace)
                    .strong()
                    .color(codex_usage_color(remaining)),
            );
        });
    });
    ui.add_space(8.0);
    let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
    ui.painter().rect_filled(bar, 2.0, border());
    if remaining > 0 {
        let fill = egui::Rect::from_min_size(
            bar.min,
            Vec2::new(bar.width() * f32::from(remaining) / 100.0, bar.height()),
        );
        ui.painter()
            .rect_filled(fill, 2.0, codex_usage_color(remaining));
    }
    if let Some(resets_at) = window.resets_at {
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("Resets {}", codex_reset_label(resets_at)))
                .size(12.0)
                .color(text_disabled()),
        );
    }
}

fn codex_window_label(duration_mins: Option<i64>) -> String {
    match duration_mins {
        Some(300) => "5-hour limit".to_owned(),
        Some(10_080) => "Weekly limit".to_owned(),
        Some(minutes) if minutes > 0 && minutes % 1_440 == 0 => {
            format!("{}-day limit", minutes / 1_440)
        }
        Some(minutes) if minutes > 0 && minutes % 60 == 0 => {
            format!("{}-hour limit", minutes / 60)
        }
        _ => "Rolling limit".to_owned(),
    }
}

fn codex_reset_label(resets_at: i64) -> String {
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(i64::MAX);
    let remaining = resets_at.saturating_sub(now);
    if remaining <= 0 {
        return "now".to_owned();
    }
    let days = remaining / 86_400;
    let hours = (remaining % 86_400) / 3_600;
    let minutes = (remaining % 3_600) / 60;
    if days > 0 {
        format!("in {days}d {hours}h")
    } else if hours > 0 {
        format!("in {hours}h {minutes}m")
    } else {
        format!("in {}m", minutes.max(1))
    }
}

fn codex_plan_label(plan: &str) -> &str {
    match plan {
        "free" => "Free",
        "go" => "Go",
        "plus" => "Plus",
        "pro" => "Pro",
        "prolite" => "Pro Lite",
        "team" => "Team",
        "business" | "self_serve_business_usage_based" => "Business",
        "enterprise" | "enterprise_cbp_usage_based" => "Enterprise",
        "edu" => "Edu",
        _ => "Codex",
    }
}

fn codex_credits_label(balance: &str) -> String {
    balance
        .parse::<f64>()
        .map_or_else(|_| balance.to_owned(), |balance| format!("{balance:.2}"))
}

fn paint_codex_mark(ui: &mut egui::Ui, available_rect: egui::Rect, color: Color32, scale: f32) {
    let logo_rect =
        egui::Rect::from_center_size(available_rect.center(), Vec2::splat(14.0 * scale));
    ui.put(
        logo_rect,
        egui::Image::from_bytes("bytes://openai/chatgpt-logo.svg", CHATGPT_LOGO_SVG)
            .tint(color)
            .sense(Sense::hover()),
    );
}

fn paint_close_icon(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    let stroke = Stroke::new(1.0, color);
    painter.line_segment(
        [center - Vec2::splat(4.0), center + Vec2::splat(4.0)],
        stroke,
    );
    painter.line_segment(
        [center + Vec2::new(-4.0, 4.0), center + Vec2::new(4.0, -4.0)],
        stroke,
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
            paint_close_icon(ui.painter(), center, text_primary());
        }
    }
    response
}

fn paint_update_icon(ui: &mut egui::Ui, installing: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
    if installing {
        ui.put(rect.shrink(6.0), egui::Spinner::new().size(16.0));
        return;
    }
    let center = rect.center();
    let stroke = Stroke::new(1.25, text_primary());
    ui.painter()
        .circle_stroke(center, 12.0, Stroke::new(1.0, border_hover()));
    ui.painter().line_segment(
        [center + Vec2::new(0.0, -5.0), center + Vec2::new(0.0, 4.0)],
        stroke,
    );
    ui.painter().line_segment(
        [center + Vec2::new(-3.5, 1.0), center + Vec2::new(0.0, 4.5)],
        stroke,
    );
    ui.painter().line_segment(
        [center + Vec2::new(3.5, 1.0), center + Vec2::new(0.0, 4.5)],
        stroke,
    );
}

fn update_version_badge(ui: &mut egui::Ui, version: &str) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(26, 26, 26))
        .stroke(Stroke::new(1.0, border()))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("v{version}"))
                    .font(FontId::monospace(10.0))
                    .color(text_secondary()),
            );
        });
}

fn update_notice_button(ui: &mut egui::Ui, label: &str, primary: bool) -> egui::Response {
    let width = if primary { 142.0 } else { 68.0 };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let hovered = response.hovered() || response.has_focus();
    let pressed = response.is_pointer_button_down_on();
    let (fill, text, stroke) = if primary {
        (
            if pressed {
                Color32::from_rgb(205, 205, 205)
            } else if hovered {
                Color32::WHITE
            } else {
                text_primary()
            },
            Color32::BLACK,
            Color32::WHITE,
        )
    } else {
        (
            if pressed {
                surface_active()
            } else if hovered {
                surface_hover()
            } else {
                Color32::BLACK
            },
            text_primary(),
            if hovered { border_hover() } else { border() },
        )
    };
    ui.painter().rect(
        rect,
        6.0,
        fill,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.0),
        text,
    );
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
    FocusTerminalLeft,
    FocusTerminalRight,
    FocusTerminalUp,
    FocusTerminalDown,
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

const PALETTE_COMMANDS: [PaletteEntry; 12] = [
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
    PaletteEntry {
        label: "Focus Terminal Left",
        command: PaletteCommand::FocusTerminalLeft,
        shortcut: "Ctrl Shift ←",
        icon: "←",
        group: PaletteGroup::Navigation,
    },
    PaletteEntry {
        label: "Focus Terminal Right",
        command: PaletteCommand::FocusTerminalRight,
        shortcut: "Ctrl Shift →",
        icon: "→",
        group: PaletteGroup::Navigation,
    },
    PaletteEntry {
        label: "Focus Terminal Up",
        command: PaletteCommand::FocusTerminalUp,
        shortcut: "Ctrl Shift ↑",
        icon: "↑",
        group: PaletteGroup::Navigation,
    },
    PaletteEntry {
        label: "Focus Terminal Down",
        command: PaletteCommand::FocusTerminalDown,
        shortcut: "Ctrl Shift ↓",
        icon: "↓",
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

#[derive(Clone, Copy, Eq, PartialEq)]
enum SettingsSection {
    General,
    Appearance,
    Keyboard,
    Advanced,
}

impl SettingsSection {
    const ALL: [Self; 4] = [
        Self::General,
        Self::Appearance,
        Self::Keyboard,
        Self::Advanced,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Keyboard => "Keyboard",
            Self::Advanced => "Advanced",
        }
    }
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
}

struct CompactSidebarResult {
    action: Option<WorkspaceAction>,
    context_menu_open: bool,
    create_workspace: bool,
    open_settings: bool,
}

fn compact_sidebar_rail(
    ui: &mut egui::Ui,
    workspaces: &[WorkspaceState],
    active_workspace: usize,
    settings_open: bool,
) -> CompactSidebarResult {
    let mut result = CompactSidebarResult {
        action: None,
        context_menu_open: false,
        create_workspace: false,
        open_settings: false,
    };
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
        result.create_workspace = true;
    }
    ui.add_space(6.0);

    let list_height = (ui.available_height() - 48.0).max(0.0);
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), list_height),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (index, workspace) in workspaces.iter().enumerate() {
                        if let Some(next) = compact_workspace_item(
                            ui,
                            workspace,
                            index,
                            index == active_workspace,
                            &mut result.context_menu_open,
                        ) {
                            result.action = Some(next);
                        }
                    }
                });
        },
    );
    ui.add_space(7.0);
    if compact_sidebar_settings_button(ui, settings_open)
        .on_hover_text("Settings")
        .clicked()
    {
        result.open_settings = true;
    }
    result
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
        egui::pos2(rect.center().x, rect.top() + 18.0),
        Vec2::splat(26.0),
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

    if response.hovered() && !response.context_menu_opened() {
        show_workspace_hover_card(ui, rect, workspace);
    }
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

#[allow(clippy::too_many_lines)]
fn show_workspace_hover_card(ui: &egui::Ui, anchor: egui::Rect, workspace: &WorkspaceState) {
    let context = ui.ctx();
    let content_rect = context.content_rect();
    let width = 326.0_f32.min((content_rect.width() - anchor.right() - 16.0).max(224.0));
    let height = 128.0;
    let mut position = egui::pos2(anchor.right() + 10.0, anchor.top() - 2.0);
    if position.x + width > content_rect.right() - 8.0 {
        position.x = anchor.left() - width - 10.0;
    }
    if position.y + height > content_rect.bottom() - 8.0 {
        position.y = content_rect.bottom() - height - 8.0;
    }
    position.y = position.y.max(content_rect.top() + 8.0);

    let summary = workspace_hover_summary(workspace);
    egui::Area::new(egui::Id::new(("workspace-hover-card", workspace.model.id)))
        .order(egui::Order::Tooltip)
        .fixed_pos(position)
        .show(context, |ui| {
            ui.set_width(width);
            ui.set_height(height);
            egui::Frame::NONE
                .fill(Color32::from_rgb(10, 10, 10))
                .stroke(Stroke::new(1.0, vercel_border()))
                .corner_radius(10.0)
                .shadow(egui::epaint::Shadow {
                    offset: [0, 10],
                    blur: 32,
                    spread: 0,
                    color: Color32::from_black_alpha(210),
                })
                .inner_margin(egui::Margin::same(0))
                .show(ui, |ui| {
                    ui.set_min_size(Vec2::new(width, height));
                    let inner = ui.max_rect().shrink2(Vec2::new(14.0, 16.0));
                    let painter = ui.painter().with_clip_rect(inner);
                    let title_y = inner.top() + 10.0;
                    paint_hover_folder_icon(
                        &painter,
                        egui::pos2(inner.left() + 8.0, title_y),
                        vercel_text_secondary(),
                    );
                    painter.text(
                        egui::pos2(inner.left() + 28.0, title_y),
                        egui::Align2::LEFT_CENTER,
                        compact_text(&workspace.model.name, 30),
                        FontId::proportional(13.5),
                        vercel_text_primary(),
                    );

                    let terminals_y = title_y + 27.0;
                    paint_hover_terminal_icon(
                        &painter,
                        egui::pos2(inner.left() + 8.0, terminals_y),
                        vercel_text_secondary(),
                    );
                    painter.text(
                        egui::pos2(inner.left() + 28.0, terminals_y),
                        egui::Align2::LEFT_CENTER,
                        format!(
                            "{} {}",
                            summary.active_terminals,
                            pluralize("terminal", summary.active_terminals)
                        ),
                        FontId::proportional(12.8),
                        vercel_text_primary(),
                    );
                    let codex_rect = egui::Rect::from_center_size(
                        egui::pos2(inner.left() + 112.0, terminals_y),
                        Vec2::splat(13.0),
                    );
                    paint_codex_mark(ui, codex_rect, vercel_text_secondary(), 0.82);
                    painter.text(
                        egui::pos2(codex_rect.right() + 5.0, terminals_y),
                        egui::Align2::LEFT_CENTER,
                        summary.codex_agents.to_string(),
                        FontId::proportional(12.5),
                        vercel_text_secondary(),
                    );

                    let opencode_rect = egui::Rect::from_center_size(
                        egui::pos2(inner.left() + 164.0, terminals_y),
                        Vec2::splat(13.0),
                    );
                    paint_opencode_mark(ui, opencode_rect);
                    painter.text(
                        egui::pos2(opencode_rect.right() + 5.0, terminals_y),
                        egui::Align2::LEFT_CENTER,
                        summary.opencode_agents.to_string(),
                        FontId::proportional(12.5),
                        vercel_text_secondary(),
                    );

                    let first_divider_y = terminals_y + 22.0;
                    painter.hline(
                        inner.x_range(),
                        first_divider_y,
                        Stroke::new(1.0, vercel_border()),
                    );

                    let path_y = first_divider_y + 28.0;
                    paint_hover_path_icon(
                        &painter,
                        egui::pos2(inner.left() + 8.0, path_y),
                        vercel_text_secondary(),
                    );
                    painter.text(
                        egui::pos2(inner.left() + 28.0, path_y),
                        egui::Align2::LEFT_CENTER,
                        compact_text(&workspace.model.root_directory.display().to_string(), 44),
                        FontId::proportional(12.8),
                        vercel_text_primary(),
                    );
                });
        });
}

struct WorkspaceHoverSummary {
    active_terminals: usize,
    codex_agents: usize,
    opencode_agents: usize,
}

fn workspace_hover_summary(workspace: &WorkspaceState) -> WorkspaceHoverSummary {
    let mut summary = WorkspaceHoverSummary {
        active_terminals: 0,
        codex_agents: 0,
        opencode_agents: 0,
    };
    for pane in workspace.panes.values() {
        if !matches!(
            pane.status,
            SessionStatus::Starting | SessionStatus::Running
        ) {
            continue;
        }
        summary.active_terminals += 1;
        match pane_agent_kind(pane) {
            Some(AgentKind::Codex) => summary.codex_agents += 1,
            Some(AgentKind::OpenCode) => summary.opencode_agents += 1,
            None => {}
        }
    }
    summary
}

enum AgentKind {
    Codex,
    OpenCode,
}

fn pane_agent_kind(pane: &TerminalPane) -> Option<AgentKind> {
    let label = pane.process_label.to_ascii_lowercase();
    if agent_text_matches_opencode(&label) {
        Some(AgentKind::OpenCode)
    } else if agent_text_matches_codex(&label) {
        Some(AgentKind::Codex)
    } else {
        None
    }
}

fn agent_text_matches_opencode(text: &str) -> bool {
    text.contains("opencode") || text.contains("open-code") || text.contains("open code")
}

fn agent_text_matches_codex(text: &str) -> bool {
    text.contains("openai codex") || text.contains(" codex") || text.starts_with("codex")
}

fn pluralize(word: &'static str, count: usize) -> &'static str {
    if count == 1 {
        word
    } else {
        match word {
            "terminal" => "terminals",
            _ => word,
        }
    }
}

fn paint_workspace_icon(ui: &egui::Ui, rect: egui::Rect, workspace: &WorkspaceState) {
    const GRID_SIZE: u8 = 10;
    const BACKGROUND: Color32 = Color32::from_rgb(0x07, 0x09, 0x16);

    let painter = ui.painter();
    let seed = workspace_identity_hash(workspace.model.id);
    let pattern = workspace_dither_pattern(seed);

    // Keep the tile itself and every dither mark square. The grid is painted
    // directly at display scale so its edges stay crisp instead of being filtered.
    painter.rect_filled(rect, 0.0, BACKGROUND);

    let inset = rect.width() * 0.03;
    let grid_rect = rect.shrink(inset);
    let step = grid_rect.width() / f32::from(GRID_SIZE);
    let pixel_size = step * 0.66;
    let pixels_per_point = ui.ctx().pixels_per_point();
    let snap = |value: f32| (value * pixels_per_point).round() / pixels_per_point;

    for row in 0..GRID_SIZE {
        for column in 0..GRID_SIZE {
            let pattern_index = usize::from(row) * usize::from(GRID_SIZE) + usize::from(column);
            let Some(color) = pattern[pattern_index] else {
                continue;
            };
            let center = egui::pos2(
                grid_rect.left() + (f32::from(column) + 0.5) * step,
                grid_rect.top() + (f32::from(row) + 0.5) * step,
            );
            let min = egui::pos2(
                snap(center.x - pixel_size * 0.5),
                snap(center.y - pixel_size * 0.5),
            );
            let size = snap(pixel_size).max(1.0 / pixels_per_point);
            painter.rect_filled(
                egui::Rect::from_min_size(min, Vec2::splat(size)),
                0.0,
                color,
            );
        }
    }
}

fn workspace_identity_hash(workspace_id: ade_core::WorkspaceId) -> u32 {
    workspace_id
        .to_string()
        .bytes()
        .fold(2_166_136_261_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        })
}

fn workspace_dither_pattern(seed: u32) -> [Option<Color32>; 100] {
    const BAYER_4X4: [u8; 16] = [
        0, 8, 2, 10, //
        12, 4, 14, 6, //
        3, 11, 1, 9, //
        15, 7, 13, 5,
    ];
    const PINK: Color32 = Color32::from_rgb(0xff, 0x38, 0x83);
    const WHITE: Color32 = Color32::from_rgb(0xf8, 0xfa, 0xff);

    let mut tones = [0.0; 100];
    let mut minimum = f32::INFINITY;
    let mut maximum = f32::NEG_INFINITY;

    for row in 0_u8..10 {
        for column in 0_u8..10 {
            let index = usize::from(row) * 10 + usize::from(column);
            let broad = workspace_value_noise(seed, column, row, 5);
            let detail = workspace_value_noise(seed ^ 0xa53a_9e37, column, row, 3);
            let tone = broad * 0.7 + detail * 0.3;
            tones[index] = tone;
            minimum = minimum.min(tone);
            maximum = maximum.max(tone);
        }
    }

    // Normalize before thresholding so each workspace keeps a useful balance of
    // ink and negative space regardless of the random field's original range.
    let range = (maximum - minimum).max(f32::EPSILON);
    let mut pattern = [None; 100];
    let mut highlight_index = 0;
    let mut highlight_score = f32::NEG_INFINITY;

    for row in 0_u8..10 {
        for column in 0_u8..10 {
            let index = usize::from(row) * 10 + usize::from(column);
            let normalized_tone = (tones[index] - minimum) / range;
            let bayer_index = usize::from((row % 4) * 4 + column % 4);
            let ordered_threshold = (f32::from(BAYER_4X4[bayer_index]) + 0.5) / 16.0;
            let score = normalized_tone - (0.12 + ordered_threshold * 0.84);
            if score > 0.0 {
                pattern[index] = Some(PINK);
                if score > highlight_score {
                    highlight_score = score;
                    highlight_index = index;
                }
            }
        }
    }

    pattern[highlight_index] = Some(WHITE);
    pattern
}

fn workspace_value_noise(seed: u32, column: u8, row: u8, scale: u8) -> f32 {
    let grid_x = column / scale;
    let grid_y = row / scale;
    let local_x = workspace_smoothstep(f32::from(column % scale) / f32::from(scale));
    let local_y = workspace_smoothstep(f32::from(row % scale) / f32::from(scale));

    let top = egui::lerp(
        workspace_noise_anchor(seed, grid_x, grid_y)
            ..=workspace_noise_anchor(seed, grid_x + 1, grid_y),
        local_x,
    );
    let bottom = egui::lerp(
        workspace_noise_anchor(seed, grid_x, grid_y + 1)
            ..=workspace_noise_anchor(seed, grid_x + 1, grid_y + 1),
        local_x,
    );
    egui::lerp(top..=bottom, local_y)
}

fn workspace_noise_anchor(seed: u32, column: u8, row: u8) -> f32 {
    let hash = mix_u32(
        seed ^ u32::from(column).wrapping_mul(0x9e37_79b9)
            ^ u32::from(row).wrapping_mul(0x85eb_ca6b),
    );
    f32::from(hash.to_le_bytes()[0]) / 255.0
}

fn workspace_smoothstep(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

fn mix_u32(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^ (value >> 16)
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
        Vec2::splat(28.0),
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

fn settings_nav_item(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    if selected || response.hovered() || response.has_focus() {
        let fill = if selected {
            vercel_surface()
        } else {
            vercel_surface_hover()
        };
        ui.painter()
            .rect_filled(rect.shrink2(Vec2::new(0.0, 1.0)), 6.0, fill);
        ui.painter().rect_stroke(
            rect.shrink2(Vec2::new(0.0, 1.0)),
            6.0,
            Stroke::new(1.0, vercel_border()),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(11.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        if selected {
            vercel_text_primary()
        } else {
            vercel_text_secondary()
        },
    );
    response
}

fn paint_settings_version_footer(ui: &egui::Ui, sidebar_rect: egui::Rect, version: &str) {
    let text = format!("Version {version}");
    ui.painter().text(
        egui::pos2(sidebar_rect.left() + 29.0, sidebar_rect.bottom() - 24.0),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(12.0),
        vercel_text_secondary(),
    );
}

fn settings_section_content(
    ui: &mut egui::Ui,
    section: SettingsSection,
    auto_expand_sidebar: &mut bool,
) {
    if !matches!(section, SettingsSection::General) {
        return;
    }

    ui.label(
        RichText::new("Sidebar")
            .font(FontId::proportional(14.0))
            .strong()
            .color(vercel_text_primary()),
    );
    ui.add_space(12.0);
    settings_toggle_row(
        ui,
        "Auto expand sidebar",
        "When enabled, hovering the collapsed sidebar opens the full workspace list.",
        auto_expand_sidebar,
    );
}

fn settings_toggle_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    value: &mut bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 72.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *value, title)
    });
    if response.clicked() {
        *value = !*value;
    }

    let row_rect = rect.shrink2(Vec2::new(0.0, 1.0));
    ui.painter().rect_filled(row_rect, 8.0, vercel_surface());
    ui.painter().rect_stroke(
        row_rect,
        8.0,
        Stroke::new(
            1.0,
            if response.hovered() {
                border_hover()
            } else {
                vercel_border()
            },
        ),
        egui::StrokeKind::Inside,
    );

    let text_x = row_rect.left() + 16.0;
    let text_clip = egui::Rect::from_min_max(
        egui::pos2(row_rect.left() + 12.0, row_rect.top()),
        egui::pos2(row_rect.right() - 74.0, row_rect.bottom()),
    );
    let painter = ui.painter().with_clip_rect(text_clip);
    painter.text(
        egui::pos2(text_x, row_rect.top() + 22.0),
        egui::Align2::LEFT_CENTER,
        title,
        FontId::proportional(14.0),
        vercel_text_primary(),
    );
    painter.text(
        egui::pos2(text_x, row_rect.top() + 45.0),
        egui::Align2::LEFT_CENTER,
        description,
        FontId::proportional(12.5),
        vercel_text_secondary(),
    );

    let toggle_rect = egui::Rect::from_center_size(
        egui::pos2(row_rect.right() - 34.0, row_rect.center().y),
        Vec2::new(38.0, 22.0),
    );
    paint_vercel_toggle(ui.painter(), toggle_rect, *value);
    response
}

fn paint_vercel_toggle(painter: &egui::Painter, rect: egui::Rect, enabled: bool) {
    let fill = if enabled {
        vercel_text_primary()
    } else {
        vercel_surface_hover()
    };
    let stroke = if enabled {
        vercel_text_primary()
    } else {
        vercel_border()
    };
    painter.rect_filled(rect, 11.0, fill);
    painter.rect_stroke(
        rect,
        11.0,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    let knob_x = if enabled {
        rect.right() - 11.0
    } else {
        rect.left() + 11.0
    };
    painter.circle_filled(
        egui::pos2(knob_x, rect.center().y),
        7.0,
        if enabled {
            vercel_bg()
        } else {
            vercel_text_secondary()
        },
    );
}

fn sidebar_settings_button(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Settings")
    });
    let button_rect = egui::Rect::from_min_size(rect.left_top(), Vec2::splat(32.0));
    if active || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            button_rect,
            6.0,
            if active {
                surface_active()
            } else {
                surface_hover()
            },
        );
    }
    paint_settings_gear(ui, button_rect, text_secondary(), 15.0);
    response
}

fn compact_sidebar_settings_button(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Settings")
    });
    let button_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(30.0));
    if active || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            button_rect,
            6.0,
            if active {
                surface_active()
            } else {
                surface_hover()
            },
        );
    }
    paint_settings_gear(ui, button_rect, text_secondary(), 15.0);
    response
}

fn topbar_settings_button(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Settings")
    });
    if active || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            6.0,
            if active {
                surface_active()
            } else {
                surface_hover()
            },
        );
    }
    paint_settings_gear(ui, rect, text_secondary(), 14.0);
    response
}

fn paint_settings_gear(ui: &mut egui::Ui, available_rect: egui::Rect, color: Color32, size: f32) {
    let icon_rect = egui::Rect::from_center_size(available_rect.center(), Vec2::splat(size));
    ui.put(
        icon_rect,
        egui::Image::from_bytes("bytes://termy/settings-gear.svg", SETTINGS_GEAR_SVG)
            .tint(color)
            .sense(Sense::hover()),
    );
}

fn paint_opencode_mark(ui: &mut egui::Ui, available_rect: egui::Rect) {
    ui.put(
        available_rect,
        egui::Image::from_bytes("bytes://opencode/logo-square.svg", OPENCODE_LOGO_SVG)
            .sense(Sense::hover()),
    );
}

fn paint_hover_folder_icon(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    let rect = egui::Rect::from_center_size(center, Vec2::new(14.0, 11.0));
    let stroke = Stroke::new(1.1, color);
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.top() + 2.5),
            egui::pos2(rect.left() + 5.0, rect.top() + 2.5),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(rect.left() + 5.0, rect.top() + 2.5),
            egui::pos2(rect.left() + 7.0, rect.top() + 4.5),
        ],
        stroke,
    );
    painter.rect_stroke(
        egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.top() + 4.0),
            egui::pos2(rect.right(), rect.bottom()),
        ),
        2.0,
        stroke,
        egui::StrokeKind::Inside,
    );
}

fn paint_hover_terminal_icon(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    let stroke = Stroke::new(1.1, color);
    let rect = egui::Rect::from_center_size(center, Vec2::new(14.0, 12.0));
    painter.rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Inside);
    painter.line_segment(
        [
            egui::pos2(rect.left() + 3.0, rect.center().y),
            egui::pos2(rect.left() + 5.0, rect.center().y + 2.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(rect.left() + 3.0, rect.center().y + 4.0),
            egui::pos2(rect.left() + 8.0, rect.center().y + 4.0),
        ],
        stroke,
    );
}

fn paint_hover_path_icon(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    let stroke = Stroke::new(1.1, color);
    let rect = egui::Rect::from_center_size(center, Vec2::splat(13.0));
    painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
    painter.line_segment(
        [
            egui::pos2(rect.left() + 2.0, rect.top() + 4.0),
            egui::pos2(rect.right() - 2.0, rect.top() + 4.0),
        ],
        stroke,
    );
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

fn modal_danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(64.0, 32.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let fill = if response.is_pointer_button_down_on() {
        Color32::from_rgb(180, 18, 32)
    } else if response.hovered() || response.has_focus() {
        Color32::from_rgb(200, 22, 38)
    } else {
        danger()
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
        Color32::WHITE,
    );
    response
}

struct TerminalPane {
    id: PaneId,
    process_label: String,
    parser: vt100::Parser,
    status: SessionStatus,
    columns: u16,
    rows: u16,
    selection: Option<TerminalSelection>,
    mouse_buttons: u8,
    pending_output: Vec<u8>,
    synchronized_output_since: Option<Instant>,
    recent_command: Option<String>,
    cursor_last_activity: Instant,
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct TerminalSizing {
    font_size: f32,
    side_padding: f32,
    bottom_padding: f32,
    footer_height: f32,
}

fn terminal_sizing(rect: egui::Rect) -> TerminalSizing {
    if rect.width() < 520.0 || rect.height() < 320.0 {
        TerminalSizing {
            font_size: 14.5,
            side_padding: 8.0,
            bottom_padding: 8.0,
            footer_height: 26.0,
        }
    } else if rect.width() < 900.0 || rect.height() < 520.0 {
        TerminalSizing {
            font_size: 15.5,
            side_padding: 12.0,
            bottom_padding: 12.0,
            footer_height: 28.0,
        }
    } else {
        TerminalSizing {
            font_size: 17.0,
            side_padding: 16.0,
            bottom_padding: 16.0,
            footer_height: 30.0,
        }
    }
}

impl TerminalPane {
    fn new(metadata: &PaneSnapshot) -> Self {
        let (git_result_sender, git_results) = unbounded();
        Self {
            id: metadata.id,
            process_label: metadata.process_label.clone(),
            parser: vt100::Parser::new(metadata.rows, metadata.cols, SCROLLBACK_LINES),
            status: metadata.status.clone(),
            columns: metadata.cols,
            rows: metadata.rows,
            selection: None,
            mouse_buttons: 0,
            pending_output: Vec::new(),
            synchronized_output_since: None,
            recent_command: None,
            cursor_last_activity: Instant::now(),
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
        self.process_label.clone_from(&metadata.process_label);
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
        if !data.is_empty() {
            self.cursor_last_activity = Instant::now();
        }
        self.pending_output.extend_from_slice(data);
        self.process_complete_output_frames();
    }

    fn handle_recent_command_osc(&mut self) {
        use base64::Engine;
        loop {
            let Some(start) = find_bytes(&self.pending_output, RECENT_COMMAND_OSC_PREFIX) else {
                return;
            };
            let value_start = start + RECENT_COMMAND_OSC_PREFIX.len();
            let rest = &self.pending_output[value_start..];
            let end_pos = rest
                .iter()
                .position(|&byte| byte == 0x07)
                .or_else(|| rest.windows(2).position(|window| window == b"\x1b\\"));
            let Some(end_offset) = end_pos else {
                return;
            };
            let encoded = &rest[..end_offset];
            let sequence_end = value_start
                + end_offset
                + if rest.get(end_offset..end_offset + 2) == Some(b"\x1b\\") {
                    2
                } else {
                    1
                };
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded)
                && decoded.len() <= RECENT_COMMAND_LIMIT
                && let Ok(command) = String::from_utf8(decoded)
            {
                let command = command.trim();
                if !command.is_empty() {
                    self.recent_command = Some(command.to_owned());
                }
            }
            self.pending_output.drain(start..sequence_end);
        }
    }

    fn handle_osc52_clipboard(&mut self) {
        use base64::Engine;
        loop {
            let Some(start) = find_bytes(&self.pending_output, b"\x1b]52;c;") else {
                return;
            };
            let data_start = start + 7;
            let rest = &self.pending_output[data_start..];
            let end_pos = rest
                .iter()
                .position(|&b| b == 0x07)
                .or_else(|| rest.windows(2).position(|w| w == b"\x1b\\"));
            let Some(end_offset) = end_pos else {
                return;
            };
            let encoded = &rest[..end_offset];
            let seq_end = data_start
                + end_offset
                + if rest.get(end_offset..end_offset + 2) == Some(b"\x1b\\") {
                    2
                } else {
                    1
                };
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded)
                && let Ok(text) = String::from_utf8(decoded)
                && let Ok(mut clipboard) = arboard::Clipboard::new()
            {
                let _ = clipboard.set_text(&text);
            }
            self.pending_output.drain(start..seq_end);
        }
    }

    fn process_complete_output_frames(&mut self) {
        self.handle_osc52_clipboard();
        self.handle_recent_command_osc();
        loop {
            if self.synchronized_output_since.is_some() {
                if let Some(end) = find_bytes(&self.pending_output, SYNCHRONIZED_OUTPUT_END) {
                    let frame_end = end + SYNCHRONIZED_OUTPUT_END.len();
                    self.parser.process(&self.pending_output[..frame_end]);
                    self.pending_output.drain(..frame_end);
                    self.synchronized_output_since = None;
                    continue;
                }
                if self.pending_output.len() >= SYNCHRONIZED_OUTPUT_LIMIT
                    || self
                        .synchronized_output_since
                        .is_some_and(|started| started.elapsed() >= SYNCHRONIZED_OUTPUT_TIMEOUT)
                {
                    self.parser.process(&self.pending_output);
                    self.pending_output.clear();
                    self.synchronized_output_since = None;
                }
                return;
            }

            if let Some(begin) = find_bytes(&self.pending_output, SYNCHRONIZED_OUTPUT_BEGIN) {
                self.parser.process(&self.pending_output[..begin]);
                self.pending_output.drain(..begin);
                self.synchronized_output_since = Some(Instant::now());
                continue;
            }

            let retained = partial_sequence_suffix(&self.pending_output, SYNCHRONIZED_OUTPUT_BEGIN);
            let process_end = self.pending_output.len() - retained;
            self.parser.process(&self.pending_output[..process_end]);
            self.pending_output.drain(..process_end);
            return;
        }
    }

    fn flush_expired_synchronized_output(&mut self, context: &egui::Context) {
        let Some(started) = self.synchronized_output_since else {
            return;
        };
        let elapsed = started.elapsed();
        if elapsed >= SYNCHRONIZED_OUTPUT_TIMEOUT {
            self.process_complete_output_frames();
        } else {
            context.request_repaint_after(SYNCHRONIZED_OUTPUT_TIMEOUT.saturating_sub(elapsed));
        }
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
#[allow(clippy::collapsible_if, clippy::if_not_else, clippy::single_match_else)]
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
    if active {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            0.0,
            Stroke::new(1.0, active_terminal_border()),
            egui::StrokeKind::Inside,
        );
    }
    pane.flush_expired_synchronized_output(ui.ctx());
    pane.refresh_git_status();
    let sizing = terminal_sizing(rect);
    let footer_top = (rect.bottom() - sizing.footer_height).max(rect.top());
    let footer_rect = egui::Rect::from_min_max(egui::pos2(rect.left(), footer_top), rect.max);
    let content_min = egui::pos2(
        (rect.left() + sizing.side_padding).min(rect.right()),
        (rect.top() + sizing.bottom_padding).min(rect.bottom()),
    );
    let content_rect = egui::Rect::from_min_max(
        content_min,
        egui::pos2(
            (rect.right() - sizing.side_padding).max(content_min.x),
            (footer_top - sizing.bottom_padding).max(content_min.y),
        ),
    );
    let font_id = FontId::new(sizing.font_size, FontFamily::Monospace);
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
    let application_mode = terminal_application_mode(screen);
    let bottom_anchored = !application_mode && screen.scrollback() == 0;
    let block_chrome = bottom_anchored;
    let visual_end_row = terminal_visual_end_row(screen);
    let grid_origin = egui::pos2(
        snap_to_pixel(content_rect.left(), pixels_per_point),
        if bottom_anchored {
            snap_to_pixel(
                content_rect.bottom() - f32::from(visual_end_row.saturating_add(1)) * cell_height,
                pixels_per_point,
            )
        } else {
            snap_to_pixel(content_rect.top(), pixels_per_point)
        },
    );
    let current_block_top = if block_chrome {
        divider_rows.last().map(|row| {
            terminal_command_header_top(
                grid_origin.y,
                *row,
                cell_height,
                sizing.bottom_padding,
                pixels_per_point,
            )
        })
    } else {
        None
    };

    if block_chrome && let Some(current_block_top) = current_block_top {
        let dock_top = (current_block_top + 2.0 * cell_height).max(content_rect.top());
        let dock_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left(), dock_top),
            egui::pos2(rect.right(), footer_top),
        );
        ui.painter()
            .rect_filled(dock_rect, 0.0, terminal_background());
        if active {
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left(), dock_top),
                    egui::pos2(rect.left() + 2.0, footer_top),
                ),
                0.0,
                Color32::from_rgb(0, 110, 254),
            );
        }
    }

    paint_terminal_footer(ui, footer_rect, active, sizing.side_padding);

    let job = terminal_layout_job(&pane.parser, pane.selection, cell_height, sizing.font_size);
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
            let current = index + 1 == divider_rows.len() && block_chrome;
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
        paint_command_header(
            ui,
            header_rect,
            &pane.cwd,
            pane.git_status.as_ref(),
            active,
            sizing.side_padding,
        );
    }
    let response = ui.interact(
        content_rect,
        egui::Id::new(("terminal-content", pane.id)),
        Sense::click_and_drag() | Sense::hover(),
    );
    // `clicked()` also fires for Enter/Space on an egui-focused pane.
    if pane.close_started_at.is_none()
        && (response.clicked_by(egui::PointerButton::Primary) || response.drag_started())
    {
        *active_pane = Some(pane.id);
        let _ = requests.send(ClientRequest::FocusPane { pane_id: pane.id });
        response.request_focus();
    }

    let mouse_mode_active = {
        let screen = pane.parser.screen();
        screen.mouse_protocol_mode() != vt100::MouseProtocolMode::None
    };

    let selection_override = ui.ctx().input(|input| input.modifiers.shift);
    if mouse_mode_active && !selection_override {
        let events = ui.ctx().input(|input| input.events.clone());
        for event in &events {
            if let egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } = event
            {
                if modifiers.shift {
                    continue;
                }
                if pos.x < content_rect.left()
                    || pos.x > content_rect.right()
                    || pos.y < content_rect.top()
                    || pos.y > content_rect.bottom()
                {
                    continue;
                }
                if let Some(cell) = cell_at_pointer(
                    *pos,
                    content_rect,
                    grid_origin,
                    pane.rows,
                    pane.columns,
                    cell_width,
                    cell_height,
                ) {
                    let screen = pane.parser.screen();
                    let mouse_encoding = screen.mouse_protocol_encoding();
                    let btn: u8 = match button {
                        eframe::egui::PointerButton::Primary => 0,
                        eframe::egui::PointerButton::Middle => 1,
                        eframe::egui::PointerButton::Secondary => 2,
                        _ => continue,
                    };
                    let button_byte = if *pressed {
                        pane.mouse_buttons |= 1 << btn;
                        btn
                    } else {
                        pane.mouse_buttons &= !(1 << btn);
                        3
                    };
                    let col = cell.column.saturating_add(1);
                    let row = cell.row.saturating_add(1);
                    let seq = match mouse_encoding {
                        vt100::MouseProtocolEncoding::Sgr => {
                            if *pressed {
                                format!("\x1b[<{button_byte};{col};{row}M")
                            } else {
                                format!("\x1b[<{button_byte};{col};{row}m")
                            }
                        }
                        _ => {
                            let c = (col + 32).min(255) as u8;
                            let r = (row + 32).min(255) as u8;
                            format!(
                                "\x1b[M{}{}{}",
                                (button_byte + 32) as char,
                                c as char,
                                r as char,
                            )
                        }
                    };
                    pane.send(requests, seq.into_bytes());
                }
            } else if let egui::Event::PointerMoved(pos) = event {
                if pane.mouse_buttons == 0 {
                    continue;
                }
                if pos.x < content_rect.left()
                    || pos.x > content_rect.right()
                    || pos.y < content_rect.top()
                    || pos.y > content_rect.bottom()
                {
                    continue;
                }
                if let Some(cell) = cell_at_pointer(
                    *pos,
                    content_rect,
                    grid_origin,
                    pane.rows,
                    pane.columns,
                    cell_width,
                    cell_height,
                ) {
                    let screen = pane.parser.screen();
                    let mouse_encoding = screen.mouse_protocol_encoding();
                    let held = pane.mouse_buttons.trailing_zeros() as u8;
                    let button_byte = held | 32;
                    let col = cell.column.saturating_add(1);
                    let row = cell.row.saturating_add(1);
                    let seq = match mouse_encoding {
                        vt100::MouseProtocolEncoding::Sgr => {
                            format!("\x1b[<{button_byte};{col};{row}M")
                        }
                        _ => {
                            let c = (col + 32).min(255) as u8;
                            let r = (row + 32).min(255) as u8;
                            format!(
                                "\x1b[M{}{}{}",
                                (button_byte + 32) as char,
                                c as char,
                                r as char,
                            )
                        }
                    };
                    pane.send(requests, seq.into_bytes());
                }
            }
        }
    } else {
        if pane.close_started_at.is_none() && response.clicked_by(egui::PointerButton::Primary) {
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
    }

    let recent_command = (block_chrome
        && cursor_column <= 2
        && current_block_top.is_some()
        && pane.close_started_at.is_none())
    .then(|| pane.recent_command.clone())
    .flatten();
    if let Some(command) = recent_command.as_deref()
        && paint_recent_command_suggestion(
            ui,
            pane.id,
            content_rect,
            grid_origin,
            cursor_row,
            cursor_column,
            cell_width,
            cell_height,
            sizing.font_size,
            command,
            active,
        )
        && active
        && accept_input
    {
        pane.send(requests, command.as_bytes().to_vec());
    }

    if active && !screen.hide_cursor() {
        let (cursor_opacity, repaint_after) =
            terminal_cursor_animation(pane.cursor_last_activity.elapsed());
        ui.ctx().request_repaint_after(repaint_after);
        let cursor_rect = terminal_cursor_rect(
            grid_origin,
            cursor_row,
            cursor_column,
            cell_width,
            cell_height,
            pixels_per_point,
        )
        .intersect(content_rect);
        if cursor_opacity > 0.0 {
            ui.painter().rect_filled(
                cursor_rect,
                0.0,
                text_primary().gamma_multiply(cursor_opacity),
            );
        }
    }

    if response.hovered() {
        let scroll = ui.ctx().input(|input| input.smooth_scroll_delta().y);
        if scroll.abs() > f32::EPSILON {
            let screen = pane.parser.screen();
            let mouse_mode = screen.mouse_protocol_mode();
            let mouse_encoding = screen.mouse_protocol_encoding();
            let _ = screen;

            if mouse_mode != vt100::MouseProtocolMode::None {
                let pointer = response
                    .interact_pointer_pos()
                    .or_else(|| ui.ctx().input(|input| input.pointer.hover_pos()));
                if let Some(pointer) = pointer {
                    if let Some(cell) = cell_at_pointer(
                        pointer,
                        content_rect,
                        grid_origin,
                        pane.rows,
                        pane.columns,
                        cell_width,
                        cell_height,
                    ) {
                        let lines = scroll_lines(scroll);
                        let col = cell.column.saturating_add(1);
                        let row = cell.row.saturating_add(1);
                        let button: u8 = if scroll > 0.0 { 64 } else { 65 };
                        for _ in 0..lines {
                            let seq = match mouse_encoding {
                                vt100::MouseProtocolEncoding::Sgr => {
                                    format!("\x1b[<{button};{col};{row}M")
                                }
                                _ => {
                                    let c = (col + 32).min(255) as u8;
                                    let r = (row + 32).min(255) as u8;
                                    format!(
                                        "\x1b[M{}{}{}",
                                        (button + 32) as char,
                                        c as char,
                                        r as char,
                                    )
                                }
                            };
                            pane.send(requests, seq.into_bytes());
                        }
                    }
                }
            } else {
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
    }

    if active && accept_input && pane.close_started_at.is_none() {
        forward_terminal_input(ui.ctx(), pane, requests, recent_command.as_deref());
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

fn paint_terminal_footer(ui: &mut egui::Ui, rect: egui::Rect, active: bool, side_padding: f32) {
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
    let baseline = egui::pos2(rect.left() + side_padding, rect.center().y);
    ui.painter().text(
        baseline,
        egui::Align2::LEFT_CENTER,
        "Ctrl+Shift+P",
        FontId::new(11.0, FontFamily::Monospace),
        key_color,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_recent_command_suggestion(
    ui: &mut egui::Ui,
    pane_id: PaneId,
    content_rect: egui::Rect,
    grid_origin: egui::Pos2,
    cursor_row: u16,
    cursor_column: u16,
    cell_width: f32,
    cell_height: f32,
    font_size: f32,
    command: &str,
    active: bool,
) -> bool {
    let left = grid_origin.x + f32::from(cursor_column) * cell_width + 8.0;
    let available = (content_rect.right() - left - 8.0).max(0.0);
    if available < 96.0 {
        return false;
    }
    let max_command_chars =
        usize::from(cells_for_pixels((available - 76.0).max(0.0), cell_width).max(4));
    let command = compact_command_suggestion(command, max_command_chars);
    let font = FontId::new(font_size, FontFamily::Monospace);
    let run_width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap("Run ".to_owned(), font.clone(), text_secondary())
            .size()
            .x
    });
    let command_width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(command.clone(), font.clone(), text_primary())
            .size()
            .x
    });
    let key_width = 26.0;
    let gap = 8.0;
    let width = (run_width + command_width + gap + key_width).min(available);
    let center_y = grid_origin.y + (f32::from(cursor_row) + 0.5) * cell_height;
    let rect = egui::Rect::from_center_size(
        egui::pos2(left + width / 2.0, center_y),
        Vec2::new(width, cell_height.max(22.0)),
    )
    .intersect(content_rect);
    let response = ui.interact(
        rect,
        egui::Id::new(("recent-command-suggestion", pane_id)),
        Sense::click(),
    );
    let painter = ui.painter().with_clip_rect(content_rect);
    let muted = if active {
        Color32::from_rgb(118, 118, 118)
    } else {
        Color32::from_rgb(78, 78, 78)
    };
    painter.text(
        egui::pos2(rect.left(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Run ",
        font.clone(),
        muted,
    );
    painter.text(
        egui::pos2(rect.left() + run_width, rect.center().y),
        egui::Align2::LEFT_CENTER,
        command,
        font,
        if response.hovered() && active {
            Color32::from_rgb(205, 205, 205)
        } else {
            Color32::from_rgb(145, 145, 145)
        },
    );
    let key_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - key_width / 2.0, rect.center().y),
        Vec2::new(key_width, 19.0),
    );
    painter.rect(
        key_rect,
        4.0,
        Color32::from_rgb(13, 13, 13),
        Stroke::new(1.0, Color32::from_rgb(67, 67, 67)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        key_rect.center(),
        egui::Align2::CENTER_CENTER,
        "→",
        FontId::new(12.0, FontFamily::Monospace),
        muted,
    );
    response.clicked()
}

fn compact_command_suggestion(command: &str, max_chars: usize) -> String {
    let command = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if command.chars().count() <= max_chars {
        return command;
    }
    let keep = max_chars.saturating_sub(1);
    format!("{}…", command.chars().take(keep).collect::<String>())
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
    side_padding: f32,
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
    let left = header_rect.left() + side_padding;
    let right = header_rect.right() - side_padding;
    let available = (right - left).max(0.0);
    let max_path_width = (available * if git.is_some() { 0.48 } else { 0.72 })
        .clamp(96.0, 360.0)
        .min(available);
    let max_path_chars = ((max_path_width - 34.0) / 7.5).max(4.0) as usize;
    let path_label = compact_header_path(cwd, max_path_chars);
    let font = FontId::new(12.0, FontFamily::Monospace);
    let stats_font = FontId::new(16.0, FontFamily::Monospace);
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
                    stats_font.clone(),
                    Color32::from_rgb(175, 175, 175),
                )
                .size()
                .x,
            fonts
                .layout_no_wrap(
                    addition_label.clone(),
                    stats_font.clone(),
                    Color32::from_rgb(82, 196, 92),
                )
                .size()
                .x,
            fonts
                .layout_no_wrap(
                    deletion_label.clone(),
                    stats_font.clone(),
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
        stats_font.clone(),
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
            stats_font.clone(),
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
            stats_font,
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
    font_size: f32,
) -> LayoutJob {
    let screen = parser.screen();
    let (rows, columns) = screen.size();
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    job.keep_trailing_whitespace = true;
    job.round_output_to_gui = false;

    for row in 0..rows {
        let divider_row = is_terminal_divider_row(screen, row);
        let mut run_text = String::new();
        let mut run_format = None;
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
            let format = TextFormat {
                font_id: FontId::new(font_size, FontFamily::Monospace),
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
            };
            if run_format.as_ref() != Some(&format) {
                append_terminal_run(&mut job, &mut run_text, run_format.take());
                run_format = Some(format);
            }
            run_text.push_str(text);
        }
        append_terminal_run(&mut job, &mut run_text, run_format);
        if row + 1 < rows {
            job.append(
                "\n",
                0.0,
                TextFormat {
                    font_id: FontId::new(font_size, FontFamily::Monospace),
                    line_height: Some(cell_height),
                    color: Color32::WHITE,
                    ..Default::default()
                },
            );
        }
    }
    job
}

fn append_terminal_run(job: &mut LayoutJob, text: &mut String, format: Option<TextFormat>) {
    if let Some(format) = format
        && !text.is_empty()
    {
        job.append(text, 0.0, format);
        text.clear();
    }
}

fn terminal_divider_rows(screen: &vt100::Screen) -> Vec<u16> {
    let (rows, _) = screen.size();
    (0..rows)
        .filter(|&row| is_terminal_divider_row(screen, row))
        .collect()
}

fn terminal_application_mode(screen: &vt100::Screen) -> bool {
    screen.alternate_screen()
        || screen.application_cursor()
        || screen.application_keypad()
        || screen.bracketed_paste()
        || screen.mouse_protocol_mode() != vt100::MouseProtocolMode::None
}

fn terminal_command_header_top(
    grid_origin_y: f32,
    divider_row: u16,
    cell_height: f32,
    bottom_padding: f32,
    pixels_per_point: f32,
) -> f32 {
    snap_to_pixel(
        grid_origin_y + f32::from(divider_row.saturating_add(1)) * cell_height - bottom_padding,
        pixels_per_point,
    )
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn partial_sequence_suffix(bytes: &[u8], sequence: &[u8]) -> usize {
    (1..sequence.len())
        .rev()
        .find(|&length| bytes.ends_with(&sequence[..length]))
        .unwrap_or(0)
}

fn terminal_visual_end_row(screen: &vt100::Screen) -> u16 {
    let (rows, columns) = screen.size();
    let content_end = (0..rows)
        .rev()
        .find(|&row| {
            (0..columns).any(|column| {
                screen.cell(row, column).is_some_and(|cell| {
                    cell.has_contents()
                        || cell.inverse()
                        || !matches!(cell.bgcolor(), vt100::Color::Default)
                })
            })
        })
        .unwrap_or(0);
    // Line editors erase the active row before redrawing it. Keep that temporarily empty row in
    // the visual extent so bottom anchoring does not jump during every keystroke.
    content_end.max(screen.cursor_position().0)
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
    recent_command: Option<&str>,
) {
    let events = context.input(|input| input.events.clone());
    let has_paste = events
        .iter()
        .any(|event| matches!(event, egui::Event::Paste(text) if !text.is_empty()));
    for event in events {
        match event {
            egui::Event::Copy => {
                if let Some(text) = pane.selected_text() {
                    context.copy_text(text);
                } else if !context.input(|input| input.modifiers.shift) {
                    pane.send(requests, vec![0x03]);
                }
            }
            egui::Event::Text(text) if !text.is_empty() => {
                // Skip text events when Ctrl is held — the Key handler
                // already encodes Ctrl+letter as a control character.
                if context.input(|input| input.modifiers.ctrl) {
                    continue;
                }
                // Skip Enter/newline — the Key handler encodes Key::Enter
                // as "\r", so the Text event would be a duplicate.
                if text == "\r" || text == "\n" {
                    continue;
                }
                let mut bytes = text.into_bytes();
                if context.input(|input| input.modifiers.alt) {
                    bytes.insert(0, 0x1b);
                }
                pane.send(requests, bytes);
            }
            egui::Event::Paste(text) if !text.is_empty() => {
                pane.send(requests, pane.paste_bytes(&text));
            }
            egui::Event::Key {
                key,
                pressed: true,
                repeat: _,
                modifiers,
                ..
            } => {
                if has_paste
                    && ((modifiers.ctrl && key == Key::V)
                        || (modifiers.shift && key == Key::Insert))
                {
                    continue;
                }
                if key == Key::ArrowRight
                    && modifiers.is_none()
                    && let Some(command) = recent_command
                {
                    pane.send(requests, command.as_bytes().to_vec());
                } else if modifiers.ctrl
                    && !modifiers.shift
                    && key == Key::C
                    && pane.selection.is_some()
                {
                    if let Some(text) = pane.selected_text() {
                        context.copy_text(text);
                    }
                } else if let Some(bytes) = encode_key(key, modifiers, pane.parser.screen()) {
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

fn vercel_bg() -> Color32 {
    Color32::BLACK
}

fn vercel_surface() -> Color32 {
    Color32::from_rgb(17, 17, 17)
}

fn vercel_surface_hover() -> Color32 {
    Color32::from_rgb(24, 24, 24)
}

fn vercel_text_primary() -> Color32 {
    Color32::from_rgb(250, 250, 250)
}

fn vercel_text_secondary() -> Color32 {
    Color32::from_rgb(136, 136, 136)
}

fn vercel_border() -> Color32 {
    Color32::from_rgb(51, 51, 51)
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

fn active_terminal_border() -> Color32 {
    Color32::from_rgb(0, 112, 243)
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

fn terminal_cursor_rect(
    grid_origin: egui::Pos2,
    row: u16,
    column: u16,
    cell_width: f32,
    cell_height: f32,
    pixels_per_point: f32,
) -> egui::Rect {
    let width = cell_width.max(2.0);
    let height = (cell_height - 2.0).max(2.0).min(cell_height);
    let center = egui::pos2(
        snap_to_pixel(
            grid_origin.x + (f32::from(column) + 0.5) * cell_width,
            pixels_per_point,
        ),
        grid_origin.y + (f32::from(row) + 0.5) * cell_height,
    );
    egui::Rect::from_center_size(center, Vec2::new(width, height))
}

fn terminal_cursor_animation(elapsed: Duration) -> (f32, Duration) {
    if elapsed < TERMINAL_CURSOR_STEADY_DURATION {
        return (1.0, TERMINAL_CURSOR_STEADY_DURATION.saturating_sub(elapsed));
    }

    let phase = elapsed
        .saturating_sub(TERMINAL_CURSOR_STEADY_DURATION)
        .as_secs_f32()
        % TERMINAL_CURSOR_BLINK_PERIOD.as_secs_f32();
    let period = TERMINAL_CURSOR_BLINK_PERIOD.as_secs_f32();
    let fade_out_start = period * 0.40;
    let fade_out_end = period * 0.54;
    let fade_in_start = period * 0.78;

    if phase < fade_out_start {
        (
            1.0,
            Duration::from_secs_f32(fade_out_start - phase).max(TERMINAL_CURSOR_FRAME_INTERVAL),
        )
    } else if phase < fade_out_end {
        let progress = (phase - fade_out_start) / (fade_out_end - fade_out_start);
        (1.0 - smoothstep(progress), TERMINAL_CURSOR_FRAME_INTERVAL)
    } else if phase < fade_in_start {
        (
            0.0,
            Duration::from_secs_f32(fade_in_start - phase).max(TERMINAL_CURSOR_FRAME_INTERVAL),
        )
    } else {
        let progress = (phase - fade_in_start) / (period - fade_in_start);
        (smoothstep(progress), TERMINAL_CURSOR_FRAME_INTERVAL)
    }
}

fn smoothstep(progress: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    progress * progress * (3.0 - 2.0 * progress)
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
    fn deferred_update_waits_for_five_idle_minutes() {
        let last_activity = Instant::now();
        assert_eq!(
            deferred_update_delay(last_activity, last_activity + Duration::from_mins(1)),
            Some(Duration::from_mins(4))
        );
        assert_eq!(
            deferred_update_delay(last_activity, last_activity + UPDATE_IDLE_DURATION),
            None
        );
    }

    #[test]
    fn updater_stages_plain_executable_without_truncating_it() {
        assert_ne!(RELEASE_ASSET_NAME, "termy.exe");
        // Older official clients identify assets by this substring. Keeping it in the renamed
        // asset lets those clients safely move to the first fixed release too.
        assert!(RELEASE_ASSET_NAME.contains("termy.exe"));

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "termy-updater-test-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let download = directory.join(RELEASE_ASSET_NAME);
        std::fs::write(&download, b"MZ updater regression fixture").unwrap();

        self_update::Extract::from_source(&download)
            .extract_file(&directory, "termy.exe")
            .unwrap();

        assert_eq!(
            std::fs::read(directory.join("termy.exe")).unwrap(),
            b"MZ updater regression fixture"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn codex_usage_parser_prefers_the_codex_limit_bucket() {
        let value = serde_json::json!({
            "rateLimits": {
                "primary": { "usedPercent": 90 }
            },
            "rateLimitsByLimitId": {
                "other": {
                    "primary": { "usedPercent": 80 }
                },
                "codex": {
                    "planType": "plus",
                    "primary": {
                        "usedPercent": 27,
                        "windowDurationMins": 300,
                        "resetsAt": 1_800_000_000
                    },
                    "secondary": {
                        "usedPercent": 41,
                        "windowDurationMins": 10_080,
                        "resetsAt": 1_800_100_000
                    },
                    "credits": { "balance": "1010.9695550000" }
                }
            }
        });

        let snapshot = parse_codex_usage_snapshot(&value).unwrap();

        assert_eq!(snapshot.plan_type.as_deref(), Some("plus"));
        assert_eq!(snapshot.primary.as_ref().unwrap().used_percent, 27);
        assert_eq!(snapshot.secondary.as_ref().unwrap().used_percent, 41);
        assert_eq!(minimum_codex_remaining_percent(&snapshot), Some(59));
        assert_eq!(codex_window_label(Some(300)), "5-hour limit");
        assert_eq!(codex_window_label(Some(10_080)), "Weekly limit");
        assert_eq!(codex_credits_label("1010.9695550000"), "1010.97");
    }

    #[test]
    fn terminal_cursor_is_centered_in_its_cell() {
        let rect = terminal_cursor_rect(egui::pos2(10.0, 20.0), 2, 3, 8.0, 18.0, 1.0);

        assert_eq!(rect.center(), egui::pos2(38.0, 65.0));
        assert_eq!(rect.size(), Vec2::new(8.0, 16.0));
    }

    #[test]
    fn terminal_cursor_stays_solid_while_typing_then_blinks_smoothly() {
        let (typing_opacity, _) = terminal_cursor_animation(TERMINAL_CURSOR_STEADY_DURATION / 2);
        let (idle_opacity, _) = terminal_cursor_animation(
            TERMINAL_CURSOR_STEADY_DURATION + TERMINAL_CURSOR_BLINK_PERIOD * 13 / 20,
        );
        let (fade_opacity, repaint_after) = terminal_cursor_animation(
            TERMINAL_CURSOR_STEADY_DURATION + TERMINAL_CURSOR_BLINK_PERIOD * 47 / 100,
        );

        assert!((typing_opacity - 1.0).abs() < f32::EPSILON);
        assert!(idle_opacity.abs() < f32::EPSILON);
        assert!((0.0..1.0).contains(&fade_opacity));
        assert_eq!(repaint_after, TERMINAL_CURSOR_FRAME_INTERVAL);
    }

    #[test]
    fn terminal_sizing_grows_with_available_pane_space() {
        let compact = terminal_sizing(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(480.0, 300.0),
        ));
        let comfortable = terminal_sizing(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(720.0, 480.0),
        ));
        let spacious = terminal_sizing(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(1100.0, 700.0),
        ));

        assert!(compact.font_size < comfortable.font_size);
        assert!(comfortable.font_size < spacious.font_size);
        assert!(compact.side_padding < comfortable.side_padding);
        assert!(comfortable.side_padding < spacious.side_padding);
        assert!(compact.bottom_padding < spacious.bottom_padding);
        assert!(compact.footer_height < spacious.footer_height);
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
    fn plain_ctrl_letter_keys_encode_for_terminal_apps() {
        let parser = vt100::Parser::new(12, 80, 100);

        assert_eq!(
            encode_key(Key::A, Modifiers::CTRL, parser.screen()),
            Some(vec![0x01])
        );
        assert_eq!(
            encode_key(Key::D, Modifiers::CTRL, parser.screen()),
            Some(vec![0x04])
        );
        assert_eq!(
            encode_key(Key::P, Modifiers::CTRL, parser.screen()),
            Some(vec![0x10])
        );
    }

    #[test]
    fn cursor_motion_does_not_move_the_visual_grid() {
        let mut parser = vt100::Parser::new(12, 80, 100);
        parser.process(b"menu\r\n  first\r\n  second");
        assert_eq!(terminal_visual_end_row(parser.screen()), 2);

        parser.process(b"\x1b[1;1H\x1b[3;4H\x1b[2;2H");
        assert_eq!(parser.screen().cursor_position(), (1, 1));
        assert_eq!(terminal_visual_end_row(parser.screen()), 2);
    }

    #[test]
    fn erased_input_row_keeps_the_visual_grid_anchored() {
        let mut parser = vt100::Parser::new(12, 80, 100);
        parser.process(b"history\r\n\r\n\r\nprompt> command");
        assert_eq!(terminal_visual_end_row(parser.screen()), 3);

        parser.process(b"\r\x1b[2K");
        assert_eq!(parser.screen().cursor_position().0, 3);
        assert_eq!(terminal_visual_end_row(parser.screen()), 3);
    }

    #[test]
    fn bottom_command_input_is_centered_between_header_and_footer() {
        let grid_origin_y = 100.0;
        let cell_height = 20.0;
        let bottom_padding = 16.0;
        let header_top =
            terminal_command_header_top(grid_origin_y, 0, cell_height, bottom_padding, 1.0);
        let header_bottom = header_top + 2.0 * cell_height;
        let input_center = grid_origin_y + 3.5 * cell_height;
        let footer_top = grid_origin_y + 4.0 * cell_height + bottom_padding;

        assert!((input_center - f32::midpoint(header_bottom, footer_top)).abs() < f32::EPSILON);
    }

    #[test]
    fn tui_modes_disable_shell_block_chrome() {
        let mut parser = vt100::Parser::new(12, 80, 100);
        assert!(!terminal_application_mode(parser.screen()));

        // Line editors briefly hide the cursor while redrawing on every keystroke. Cursor
        // visibility alone must not move a bottom-anchored shell prompt to the top of the pane.
        parser.process(b"\x1b[?25l");
        assert!(!terminal_application_mode(parser.screen()));

        parser.process(b"\x1b[?25h\x1b[?2004h");
        assert!(!parser.screen().alternate_screen());
        assert!(terminal_application_mode(parser.screen()));

        parser.process(b"\x1b[?2004l\x1b[?1000h");
        assert!(terminal_application_mode(parser.screen()));

        parser.process(b"\x1b[?1000l");
        assert!(!terminal_application_mode(parser.screen()));
    }

    #[test]
    fn shell_prompt_stays_bottom_anchored_during_cursor_hidden_redraw() {
        let mut parser = vt100::Parser::new(12, 80, 100);
        parser
            .process(format!("\x1b[8m{TERMINAL_DIVIDER_MARKER}\x1b[0m\r\nPS> command").as_bytes());
        let visual_end_before = terminal_visual_end_row(parser.screen());

        parser.process(b"\x1b[?25l\r\x1b[2KPS> commandx");

        assert!(!terminal_application_mode(parser.screen()));
        assert_eq!(
            terminal_visual_end_row(parser.screen()),
            visual_end_before,
            "the redraw must not change the bottom-anchored grid extent"
        );
    }

    #[test]
    fn inline_tui_ignores_stale_shell_block_marker() {
        let mut parser = vt100::Parser::new(12, 80, 100);
        parser.process(
            format!("\x1b[8m{TERMINAL_DIVIDER_MARKER}\x1b[0m\r\nPS> codex\r\n\x1b[?2004h")
                .as_bytes(),
        );

        assert_eq!(terminal_divider_rows(parser.screen()), vec![0]);
        assert!(terminal_application_mode(parser.screen()));
        assert!(!parser.screen().alternate_screen());
    }

    #[test]
    fn synchronized_tui_output_is_applied_as_one_frame() {
        let metadata = PaneSnapshot {
            id: PaneId::new(),
            workspace_id: ade_core::WorkspaceId::new(),
            cwd: PathBuf::from("."),
            process_label: "test".to_owned(),
            cols: 80,
            rows: 12,
            status: SessionStatus::Running,
        };
        let mut pane = TerminalPane::new(&metadata);

        pane.process_output(b"\x1b[?20");
        pane.process_output(b"26hmenu\r\n  first");
        assert!(pane.parser.screen().contents().is_empty());

        pane.process_output(b"\r\n  second\x1b[?2026l");
        assert!(pane.parser.screen().contents().contains("menu"));
        assert!(pane.parser.screen().contents().contains("second"));
        assert!(pane.pending_output.is_empty());
        assert!(pane.synchronized_output_since.is_none());
    }

    #[test]
    fn terminal_layout_coalesces_cells_with_identical_formatting() {
        let mut parser = vt100::Parser::new(12, 80, 100);
        parser.process(b"one uniformly formatted line");
        let job = terminal_layout_job(&parser, None, 18.0, 14.0);

        assert!(job.sections.len() < 30, "sections: {}", job.sections.len());
    }

    #[test]
    fn recent_command_osc_is_captured_without_reaching_the_terminal_grid() {
        use base64::Engine;
        let metadata = PaneSnapshot {
            id: PaneId::new(),
            workspace_id: ade_core::WorkspaceId::new(),
            cwd: PathBuf::from("."),
            process_label: "test".to_owned(),
            cols: 80,
            rows: 12,
            status: SessionStatus::Running,
        };
        let mut pane = TerminalPane::new(&metadata);
        let command = "cargo test --workspace";
        let encoded = base64::engine::general_purpose::STANDARD.encode(command);

        pane.process_output(format!("\x1b]6973;{encoded}\x07PS> ").as_bytes());

        assert_eq!(pane.recent_command.as_deref(), Some(command));
        assert_eq!(pane.parser.screen().contents(), "PS> ");
    }

    #[test]
    fn long_recent_commands_are_compacted_for_the_suggestion_row() {
        assert_eq!(
            compact_command_suggestion("  cargo   test --workspace  ", 15),
            "cargo test --w…"
        );
        assert_eq!(compact_command_suggestion("git status", 20), "git status");
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
    fn workspace_hover_summary_counts_active_agent_terminals() {
        let mut workspace = Workspace::new("my-ADE", PathBuf::from(r"D:\NimsWorkspace\my-ADE"));
        workspace.layout = LayoutNode::Empty;
        workspace.active_pane_id = None;
        let mut panes = HashMap::new();
        for (label, status) in [
            ("codex.exe", SessionStatus::Running),
            ("opencode", SessionStatus::Starting),
            ("pwsh.exe", SessionStatus::Running),
            ("codex.exe", SessionStatus::Exited { exit_code: 0 }),
        ] {
            let metadata = PaneSnapshot {
                id: PaneId::new(),
                workspace_id: workspace.id,
                cwd: workspace.root_directory.clone(),
                process_label: label.to_owned(),
                cols: 80,
                rows: 24,
                status,
            };
            panes.insert(metadata.id, TerminalPane::new(&metadata));
        }
        let state = WorkspaceState {
            model: workspace,
            panes,
        };

        let summary = workspace_hover_summary(&state);

        assert_eq!(summary.active_terminals, 3);
        assert_eq!(summary.codex_agents, 1);
        assert_eq!(summary.opencode_agents, 1);
    }

    #[test]
    fn workspace_hover_summary_ignores_stale_terminal_text() {
        let mut workspace = Workspace::new("my-ADE", PathBuf::from(r"D:\NimsWorkspace\my-ADE"));
        workspace.layout = LayoutNode::Empty;
        workspace.active_pane_id = None;
        let mut panes = HashMap::new();
        for visible_text in [
            ">_ OpenAI Codex (v0.145.0)\r\nmodel: gpt-5.5 high",
            "opencode\r\nBuild MiMo V2.5 Free OpenCode Zen",
        ] {
            let metadata = PaneSnapshot {
                id: PaneId::new(),
                workspace_id: workspace.id,
                cwd: workspace.root_directory.clone(),
                process_label: "pwsh.exe".to_owned(),
                cols: 80,
                rows: 24,
                status: SessionStatus::Running,
            };
            let mut pane = TerminalPane::new(&metadata);
            pane.parser.process(visible_text.as_bytes());
            panes.insert(metadata.id, pane);
        }
        let state = WorkspaceState {
            model: workspace,
            panes,
        };

        let summary = workspace_hover_summary(&state);

        assert_eq!(summary.active_terminals, 2);
        assert_eq!(summary.codex_agents, 0);
        assert_eq!(summary.opencode_agents, 0);
    }

    #[test]
    fn workspace_dither_is_stable_and_uses_one_color_plus_white() {
        let workspace_id = "00000000-0000-4000-8000-000000000001"
            .parse::<ade_core::WorkspaceId>()
            .unwrap();
        let seed = workspace_identity_hash(workspace_id);
        assert_eq!(seed, workspace_identity_hash(workspace_id));

        let pattern = workspace_dither_pattern(seed);
        let cells: Vec<_> = pattern.iter().flatten().copied().collect();
        assert!((25..=75).contains(&cells.len()));
        let identity_color = Color32::from_rgb(0xff, 0x38, 0x83);
        let white = Color32::from_rgb(0xf8, 0xfa, 0xff);
        assert!(
            cells
                .iter()
                .all(|color| [identity_color, white].contains(color))
        );
        assert_eq!(cells.iter().filter(|color| **color == white).count(), 1);
        assert_eq!(pattern, workspace_dither_pattern(seed));
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
    fn pointer_mapping_accounts_for_an_offset_grid_origin() {
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
