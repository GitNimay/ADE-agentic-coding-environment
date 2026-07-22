use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ade_core::{PaneId, SessionStatus};
use ade_protocol::{
    ClientRequest, PROTOCOL_VERSION, ServerEvent, Versioned, pipe_name, read_frame, write_frame,
};
use ade_pty::{ConPtySession, PtySize, SpawnCommand};
use ade_storage::Repository;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ,
    GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_MODE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::IO::CancelIoEx;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, PeekNamedPipe,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::PCWSTR;

use crate::{DaemonError, DaemonState, StateAction, load_state};

const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const CLIENT_EVENT_CAPACITY: usize = 256;
const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
const POWERSHELL_PROMPT_HOOK: &str = r#"$global:__ade_original_prompt=$function:prompt; function global:prompt { $uri=([uri](Get-Location).ProviderPath).AbsoluteUri; [Console]::Write("$([char]27)]7;$uri$([char]7)"); [Console]::Write("$([char]27)[8m__ADE_BLOCK_DIVIDER__$([char]27)[0m`r`n"); & $global:__ade_original_prompt }"#;
const OSC_BUFFER_LIMIT: usize = 8192;

struct RuntimeSession {
    input: Sender<Vec<u8>>,
    control: Sender<Control>,
}

#[derive(Default)]
struct CwdTracker {
    buffer: Vec<u8>,
}

impl CwdTracker {
    fn process(&mut self, bytes: &[u8]) -> Option<PathBuf> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > OSC_BUFFER_LIMIT {
            let excess = self.buffer.len() - OSC_BUFFER_LIMIT;
            self.buffer.drain(..excess);
        }
        let cwd = extract_osc7_cwd(&self.buffer)?;
        self.buffer.clear();
        Some(cwd)
    }
}

enum Control {
    Resize(PtySize),
    Stop,
}

#[derive(Default)]
struct OutputHub {
    buffers: HashMap<PaneId, VecDeque<u8>>,
    client: Option<(u64, Sender<ServerEvent>)>,
    next_client_id: u64,
}

impl OutputHub {
    fn append(&mut self, pane_id: PaneId, bytes: &[u8]) {
        let buffer = self.buffers.entry(pane_id).or_default();
        if bytes.len() >= OUTPUT_LIMIT {
            buffer.clear();
            buffer.extend(&bytes[bytes.len() - OUTPUT_LIMIT..]);
        } else {
            let excess = buffer
                .len()
                .saturating_add(bytes.len())
                .saturating_sub(OUTPUT_LIMIT);
            buffer.drain(..excess);
            buffer.extend(bytes);
        }
        self.emit(ServerEvent::TerminalOutput {
            pane_id,
            data: bytes.to_vec(),
        });
    }

    fn emit(&mut self, event: ServerEvent) {
        if let Some((_, client)) = &self.client {
            let _ = client.try_send(event);
        }
    }

    fn replay_for_attach(
        &mut self,
        snapshot: ade_protocol::AppSnapshot,
    ) -> (u64, Vec<ServerEvent>) {
        self.next_client_id = self.next_client_id.wrapping_add(1);
        let client_id = self.next_client_id;
        let mut events = vec![ServerEvent::Attached { snapshot }];
        for (&pane_id, buffer) in &self.buffers {
            if !buffer.is_empty() {
                events.push(ServerEvent::TerminalOutput {
                    pane_id,
                    data: buffer.iter().copied().collect(),
                });
            }
        }
        (client_id, events)
    }

    fn activate(&mut self, client_id: u64, sender: Sender<ServerEvent>) {
        self.client = Some((client_id, sender));
    }

    fn detach(&mut self, client_id: u64) {
        if matches!(self.client, Some((id, _)) if id == client_id) {
            self.client = None;
        }
    }
}

struct Shared {
    state: Mutex<DaemonState>,
    output: Mutex<OutputHub>,
    repository: Mutex<Repository>,
    sessions: Mutex<HashMap<PaneId, RuntimeSession>>,
    requests: Mutex<()>,
    clients: Mutex<HashMap<u64, ClientHandles>>,
    next_client_id: AtomicU64,
    shutdown: AtomicBool,
    shell: PathBuf,
    process_label: String,
}

#[derive(Clone, Copy)]
struct ClientHandles {
    reader: isize,
    writer: isize,
}

pub(super) fn run() -> Result<(), DaemonError> {
    let _singleton = DaemonSingleton::acquire()?;
    let repository = Repository::open_default()?;
    let state = load_state(&repository)?;
    let shell = resolve_shell();
    let process_label = shell
        .file_name()
        .unwrap_or(shell.as_os_str())
        .to_string_lossy()
        .into_owned();
    let shared = Arc::new(Shared {
        state: Mutex::new(state),
        output: Mutex::new(OutputHub::default()),
        repository: Mutex::new(repository),
        sessions: Mutex::new(HashMap::new()),
        requests: Mutex::new(()),
        clients: Mutex::new(HashMap::new()),
        next_client_id: AtomicU64::new(0),
        shutdown: AtomicBool::new(false),
        shell,
        process_label,
    });

    let startup_panes = lock(&shared.state).snapshot().panes.clone();
    for pane in startup_panes {
        spawn_pane(&shared, pane.id)?;
    }
    persist_snapshot(&shared)?;

    let mut clients = Vec::new();
    loop {
        let pipe = accept_pipe()?;
        if shared.shutdown.load(Ordering::Acquire) {
            drop(pipe);
            break;
        }
        let client_shared = Arc::clone(&shared);
        clients.push(thread::spawn(move || {
            let _ = serve_client(pipe, &client_shared);
        }));
        clients.retain(|client| !client.is_finished());
    }
    for client in clients {
        let _ = client.join();
    }
    Ok(())
}

struct DaemonSingleton(HANDLE);

impl DaemonSingleton {
    fn acquire() -> Result<Self, DaemonError> {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".to_owned());
        let safe_user: String = user
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        let mut name: Vec<u16> = format!(r"Local\ADE-Daemon-{safe_user}")
            .encode_utf16()
            .collect();
        name.push(0);
        // SAFETY: name is a live null-terminated UTF-16 string and default security is requested.
        let handle = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr()))? };
        // SAFETY: GetLastError reads thread-local status from the immediately preceding API call.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // SAFETY: this process owns this returned handle and closes it exactly once here.
            let _ = unsafe { CloseHandle(handle) };
            return Err(DaemonError::AlreadyRunning);
        }
        Ok(Self(handle))
    }
}

impl Drop for DaemonSingleton {
    fn drop(&mut self) {
        // SAFETY: this guard uniquely owns the mutex handle.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn serve_client(pipe: File, shared: &Arc<Shared>) -> Result<(), DaemonError> {
    let client_id = shared.next_client_id.fetch_add(1, Ordering::Relaxed);
    let raw_handle = pipe.as_raw_handle() as isize;
    lock(&shared.clients).insert(
        client_id,
        ClientHandles {
            reader: raw_handle,
            writer: raw_handle,
        },
    );
    let (event_tx, event_rx) = bounded(CLIENT_EVENT_CAPACITY);
    let mut pipe = pipe;
    let mut attached_client_id = None;
    let result = if shared.shutdown.load(Ordering::Acquire) {
        Ok(())
    } else {
        client_loop(
            &mut pipe,
            shared,
            &event_tx,
            &event_rx,
            &mut attached_client_id,
        )
    };

    if let Some(output_client_id) = attached_client_id {
        lock(&shared.output).detach(output_client_id);
    }
    lock(&shared.clients).remove(&client_id);
    result
}

fn client_loop(
    reader: &mut File,
    shared: &Arc<Shared>,
    event_tx: &Sender<ServerEvent>,
    event_rx: &Receiver<ServerEvent>,
    attached_client_id: &mut Option<u64>,
) -> Result<(), DaemonError> {
    loop {
        for event in event_rx.try_iter() {
            write_frame(reader, &Versioned::new(event))?;
        }
        match pipe_bytes_available(reader) {
            Ok(0) => {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Ok(_) => {}
            Err(error) if client_disconnected(&error) => {
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        }
        let request = match read_frame::<_, Versioned<ClientRequest>>(reader) {
            Ok(request) => request,
            Err(ade_protocol::FrameError::Io(error)) if client_disconnected(&error) => {
                return Ok(());
            }
            Err(error) => {
                send_request_error(event_tx, &error);
                return Ok(());
            }
        };
        if request.protocol_version != PROTOCOL_VERSION {
            let _ = event_tx.send(ServerEvent::Error {
                message: DaemonError::ProtocolVersion {
                    received: request.protocol_version,
                    expected: PROTOCOL_VERSION,
                }
                .to_string(),
            });
            continue;
        }

        if request.message == ClientRequest::Attach {
            let snapshot = lock(&shared.state).snapshot().clone();
            let mut output = lock(&shared.output);
            let (id, replay) = output.replay_for_attach(snapshot);
            for event in replay {
                event_tx
                    .send(event)
                    .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
            }
            output.activate(id, event_tx.clone());
            *attached_client_id = Some(id);
            continue;
        }
        if request.message == ClientRequest::GetSnapshot {
            let snapshot = lock(&shared.state).snapshot().clone();
            event_tx
                .send(ServerEvent::Attached { snapshot })
                .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
            continue;
        }

        let _request = lock(&shared.requests);
        if shared.shutdown.load(Ordering::Acquire) {
            return Ok(());
        }
        let result = lock(&shared.state).handle(request.message, &shared.process_label);
        match result {
            Ok(change) => {
                let is_shutdown = matches!(change.action, StateAction::Shutdown);
                if let Err(error) = apply_action(change.action, shared) {
                    let _ = event_tx.try_send(ServerEvent::Error {
                        message: error.to_string(),
                    });
                }
                if change.persist
                    && let Err(error) = persist_snapshot(shared)
                {
                    send_request_error(event_tx, &error);
                    return Err(error);
                }
                if change.publish_snapshot {
                    let snapshot = lock(&shared.state).snapshot().clone();
                    lock(&shared.output).emit(ServerEvent::WorkspaceUpdated { snapshot });
                }
                if is_shutdown {
                    begin_shutdown(shared);
                    return Ok(());
                }
            }
            Err(error) => {
                let _ = event_tx.try_send(ServerEvent::Error {
                    message: error.to_string(),
                });
            }
        }
    }
}

fn client_disconnected(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
    )
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

fn send_request_error(event_tx: &Sender<ServerEvent>, error: &impl std::fmt::Display) {
    let _ = event_tx.try_send(ServerEvent::Error {
        message: error.to_string(),
    });
}

fn apply_action(action: StateAction, shared: &Arc<Shared>) -> Result<(), DaemonError> {
    match action {
        StateAction::None | StateAction::Shutdown => {}
        StateAction::SpawnPane(pane_id) => spawn_pane(shared, pane_id)?,
        StateAction::ClosePane(pane_id) => close_pane(shared, pane_id),
        StateAction::ClosePanes(panes) => {
            for pane_id in panes {
                close_pane(shared, pane_id);
            }
        }
        StateAction::Input { pane_id, data } => {
            lock(&shared.sessions)
                .get(&pane_id)
                .ok_or(DaemonError::PaneNotFound(pane_id))?
                .input
                .send(data)
                .map_err(|_| DaemonError::PaneNotFound(pane_id))?;
        }
        StateAction::Resize {
            pane_id,
            cols,
            rows,
        } => {
            let size = PtySize::new(cols, rows)?;
            lock(&shared.sessions)
                .get(&pane_id)
                .ok_or(DaemonError::PaneNotFound(pane_id))?
                .control
                .send(Control::Resize(size))
                .map_err(|_| DaemonError::PaneNotFound(pane_id))?;
        }
    }
    Ok(())
}

fn spawn_pane(shared: &Arc<Shared>, pane_id: PaneId) -> Result<(), DaemonError> {
    let pane = lock(&shared.state).pane(pane_id)?.clone();
    let cwd = if pane.cwd.is_dir() {
        pane.cwd
    } else {
        std::env::current_dir()?
    };
    let size = PtySize::new(pane.cols, pane.rows)?;
    let mut command = SpawnCommand::new(shared.shell.clone(), cwd);
    let shell_name = shared
        .shell
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if shell_name.eq_ignore_ascii_case("pwsh.exe")
        || shell_name.eq_ignore_ascii_case("powershell.exe")
    {
        command = command.arguments(["-NoExit", "-Command", POWERSHELL_PROMPT_HOOK]);
    }
    let mut session = match ConPtySession::spawn(&command, size) {
        Ok(session) => session,
        Err(error) => {
            let status = SessionStatus::FailedToStart {
                message: error.to_string(),
            };
            lock(&shared.state).set_pane_status(pane_id, status.clone())?;
            lock(&shared.output).emit(ServerEvent::PaneStatus { pane_id, status });
            return Ok(());
        }
    };
    let mut reader = session.take_reader()?;
    let mut writer = session.take_writer()?;
    let (input_tx, input_rx) = bounded::<Vec<u8>>(CLIENT_EVENT_CAPACITY);
    let (control_tx, control_rx) = bounded::<Control>(16);

    let output_shared = Arc::clone(shared);
    thread::spawn(move || {
        let mut buffer = [0_u8; 16 * 1024];
        let mut cwd_tracker = CwdTracker::default();
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 {
                break;
            }
            lock(&output_shared.output).append(pane_id, &buffer[..read]);
            if let Some(cwd) = cwd_tracker.process(&buffer[..read]) {
                update_pane_cwd(&output_shared, pane_id, cwd);
            }
        }
    });
    thread::spawn(move || {
        while let Ok(bytes) = input_rx.recv() {
            if writer.write_all(&bytes).is_err() {
                break;
            }
        }
    });
    let status_shared = Arc::clone(shared);
    thread::spawn(move || control_session(pane_id, session, &control_rx, &status_shared));

    lock(&shared.sessions).insert(
        pane_id,
        RuntimeSession {
            input: input_tx,
            control: control_tx,
        },
    );
    let status = SessionStatus::Running;
    lock(&shared.state).set_pane_status(pane_id, status.clone())?;
    lock(&shared.output).emit(ServerEvent::PaneStatus { pane_id, status });
    Ok(())
}

fn update_pane_cwd(shared: &Shared, pane_id: PaneId, cwd: PathBuf) {
    let changed = lock(&shared.state).set_pane_cwd(pane_id, cwd);
    if !matches!(changed, Ok(true)) {
        return;
    }
    if let Err(error) = persist_snapshot(shared) {
        lock(&shared.output).emit(ServerEvent::Error {
            message: error.to_string(),
        });
        return;
    }
    let snapshot = lock(&shared.state).snapshot().clone();
    lock(&shared.output).emit(ServerEvent::WorkspaceUpdated { snapshot });
}

fn extract_osc7_cwd(bytes: &[u8]) -> Option<PathBuf> {
    const PREFIX: &[u8] = b"\x1b]7;";
    let start = bytes
        .windows(PREFIX.len())
        .rposition(|window| window == PREFIX)?
        + PREFIX.len();
    let tail = &bytes[start..];
    let bell = tail.iter().position(|byte| *byte == 0x07);
    let string_terminator = tail.windows(2).position(|window| window == b"\x1b\\");
    let end = match (bell, string_terminator) {
        (Some(bell), Some(terminator)) => bell.min(terminator),
        (Some(bell), None) => bell,
        (None, Some(terminator)) => terminator,
        (None, None) => return None,
    };
    let uri = std::str::from_utf8(&tail[..end]).ok()?;
    let url = url::Url::parse(uri).ok()?;
    (url.scheme() == "file")
        .then(|| url.to_file_path().ok())
        .flatten()
}

fn control_session(
    pane_id: PaneId,
    mut session: ConPtySession,
    receiver: &Receiver<Control>,
    shared: &Shared,
) {
    loop {
        match receiver.recv_timeout(Duration::from_millis(200)) {
            Ok(Control::Resize(size)) => {
                if session.resize(size).is_err() {
                    break;
                }
            }
            Ok(Control::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        match session.try_wait() {
            Ok(Some(exit)) => {
                let status = SessionStatus::Exited {
                    exit_code: exit.code,
                };
                let _ = lock(&shared.state).set_pane_status(pane_id, status.clone());
                lock(&shared.output).emit(ServerEvent::PaneStatus { pane_id, status });
                return;
            }
            Ok(None) => {}
            Err(error) => {
                let status = SessionStatus::FailedToStart {
                    message: error.to_string(),
                };
                let _ = lock(&shared.state).set_pane_status(pane_id, status.clone());
                lock(&shared.output).emit(ServerEvent::PaneStatus { pane_id, status });
                return;
            }
        }
    }
}

fn close_pane(shared: &Arc<Shared>, pane_id: PaneId) {
    if let Some(session) = lock(&shared.sessions).remove(&pane_id) {
        let _ = session.control.send(Control::Stop);
    }
    lock(&shared.output).buffers.remove(&pane_id);
}

fn persist_snapshot(shared: &Shared) -> Result<(), DaemonError> {
    let snapshot = lock(&shared.state).snapshot().clone();
    lock(&shared.repository).save_snapshot(&snapshot)?;
    Ok(())
}

fn begin_shutdown(shared: &Shared) {
    if shared.shutdown.swap(true, Ordering::AcqRel) {
        return;
    }
    for (_, session) in lock(&shared.sessions).drain() {
        let _ = session.control.send(Control::Stop);
    }
    cancel_clients(shared);
    wake_accept_loop();
}

fn cancel_clients(shared: &Shared) {
    let clients = lock(&shared.clients);
    for handles in clients.values() {
        for raw in [handles.reader, handles.writer] {
            // SAFETY: client handles remain owned and registered until after this lock is released.
            let _ = unsafe { CancelIoEx(HANDLE(raw as *mut _), None) };
        }
    }
}

fn accept_pipe() -> Result<File, DaemonError> {
    let mut wide: Vec<u16> = pipe_name().encode_utf16().collect();
    wide.push(0);
    // SAFETY: the name is a live null-terminated UTF-16 buffer and all parameters are constants.
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(wide.as_ptr()),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            None,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: handle is a live named-pipe server handle. ERROR_PIPE_CONNECTED means the client
    // connected between creation and this call and is therefore also a successful connection.
    if let Err(error) = unsafe { ConnectNamedPipe(handle, None) }
        && error.code() != ERROR_PIPE_CONNECTED.to_hresult()
    {
        close_raw_handle(handle);
        return Err(error.into());
    }
    // SAFETY: ownership of the newly-created handle is transferred exactly once to File.
    Ok(unsafe { File::from_raw_handle(handle.0.cast()) })
}

fn wake_accept_loop() {
    let mut wide: Vec<u16> = pipe_name().encode_utf16().collect();
    wide.push(0);
    for _ in 0..500 {
        // SAFETY: the pipe name is null-terminated and the returned handle is owned by this call.
        let result = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        };
        match result {
            Ok(handle) => {
                close_raw_handle(handle);
                return;
            }
            Err(error) if error.code() == ERROR_PIPE_BUSY.to_hresult() => {}
            Err(_) => {}
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn close_raw_handle(handle: HANDLE) {
    // Transfer ownership to File and immediately drop it to avoid duplicating close logic.
    // SAFETY: this is only called for an owned handle that has not been transferred elsewhere.
    drop(unsafe { File::from_raw_handle(handle.0.cast()) });
}

fn resolve_shell() -> PathBuf {
    for executable in ["pwsh.exe", "powershell.exe"] {
        if let Some(path) = find_on_path(executable) {
            return path;
        }
    }
    std::env::var_os("COMSPEC").map_or_else(
        || PathBuf::from(r"C:\Windows\System32\cmd.exe"),
        PathBuf::from,
    )
}

fn find_on_path(executable: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(executable))
            .find(|candidate| candidate.is_file())
    })
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use ade_protocol::AppSnapshot;

    use super::*;

    #[test]
    fn replacing_attached_client_keeps_broadcasts_on_new_subscriber() {
        let mut output = OutputHub::default();
        let pane_id = PaneId::new();
        output.append(pane_id, b"buffered");

        let (first_tx, first_rx) = bounded(4);
        let (first_id, replay) = output.replay_for_attach(AppSnapshot::default());
        assert_eq!(replay.len(), 2);
        output.activate(first_id, first_tx);

        let (second_tx, second_rx) = bounded(4);
        let (second_id, _) = output.replay_for_attach(AppSnapshot::default());
        output.activate(second_id, second_tx);
        output.detach(first_id);
        output.emit(ServerEvent::Error {
            message: "requester-independent update".to_owned(),
        });

        assert!(first_rx.try_recv().is_err());
        assert!(matches!(
            second_rx.try_recv(),
            Ok(ServerEvent::Error { .. })
        ));
    }

    #[test]
    fn cwd_tracker_handles_split_sequences_and_uses_latest_path() {
        let mut tracker = CwdTracker::default();
        assert_eq!(tracker.process(b"before\x1b]7;file:///D:/partial"), None);
        assert_eq!(
            tracker.process(b"/path\x07middle\x1b]7;file:///D:/NimsWorkspace/my-ADE\x1b\\after"),
            Some(PathBuf::from(r"D:\NimsWorkspace\my-ADE"))
        );
    }
}
