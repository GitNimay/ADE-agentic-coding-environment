use std::ffi::{OsStr, c_void};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PtySize {
    pub columns: u16,
    pub rows: u16,
}

impl PtySize {
    /// Creates a character-cell size accepted by the Windows `COORD` type.
    ///
    /// # Errors
    ///
    /// Returns [`PtyError::InvalidSize`] when either dimension is zero or exceeds `i16::MAX`.
    pub fn new(columns: u16, rows: u16) -> Result<Self, PtyError> {
        if columns == 0 || rows == 0 || columns > i16::MAX as u16 || rows > i16::MAX as u16 {
            return Err(PtyError::InvalidSize { columns, rows });
        }
        Ok(Self { columns, rows })
    }

    fn coord(self) -> COORD {
        COORD {
            X: i16::try_from(self.columns).expect("PtySize validates columns"),
            Y: i16::try_from(self.rows).expect("PtySize validates rows"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpawnCommand {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub current_directory: PathBuf,
}

impl SpawnCommand {
    #[must_use]
    pub fn new(executable: PathBuf, current_directory: PathBuf) -> Self {
        Self {
            executable,
            arguments: Vec::new(),
            current_directory,
        }
    }

    #[must_use]
    pub fn arguments(mut self, arguments: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.arguments = arguments.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExitStatus {
    pub code: u32,
}

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("invalid terminal size {columns}x{rows}")]
    InvalidSize { columns: u16, rows: u16 },
    #[error("a command or path contains an embedded null character")]
    EmbeddedNull,
    #[error("a ConPTY operation failed: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("the ConPTY input pipe accepted zero bytes")]
    WriteZero,
    #[error("the ConPTY {0} pipe has already been taken")]
    PipeAlreadyTaken(&'static str),
    #[error("a Win32 structure size exceeded its API field")]
    StructureSizeOverflow,
    #[error("unexpected wait result {0:#x}")]
    UnexpectedWaitResult(u32),
}

pub struct ConPtySession {
    pseudo_console: PseudoConsole,
    input: Option<OwnedHandle>,
    output: Option<OwnedHandle>,
    process: OwnedHandle,
    process_id: u32,
    size: PtySize,
}

impl ConPtySession {
    /// Creates a pseudoconsole and starts `command` inside it.
    ///
    /// # Errors
    ///
    /// Returns an error when paths contain embedded nulls or a required Win32 operation fails.
    pub fn spawn(command: &SpawnCommand, size: PtySize) -> Result<Self, PtyError> {
        let (pseudo_input, host_input) = create_pipe()?;
        let (host_output, pseudo_output) = create_pipe()?;

        // SAFETY: both handles are live synchronous pipe handles and remain live through process
        // creation. The dimensions were validated by PtySize::new.
        let pseudo_console = unsafe {
            PseudoConsole(CreatePseudoConsole(
                size.coord(),
                pseudo_input.raw(),
                pseudo_output.raw(),
                0,
            )?)
        };

        let mut attributes = AttributeList::new(1)?;
        attributes.set_pseudo_console(pseudo_console.0)?;

        let mut startup_info = STARTUPINFOEXW::default();
        startup_info.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>())
            .map_err(|_| PtyError::StructureSizeOverflow)?;
        startup_info.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup_info.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
        startup_info.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
        startup_info.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
        startup_info.lpAttributeList = attributes.raw();

        let executable = to_wide_null(command.executable.as_os_str())?;
        let current_directory = to_wide_null(command.current_directory.as_os_str())?;
        let mut command_line = build_command_line(&command.executable, &command.arguments)?;
        let mut process_info = PROCESS_INFORMATION::default();

        // SAFETY: all pointers reference live, null-terminated buffers for the duration of the
        // call. startup_info owns a valid initialized attribute list containing the live HPCON.
        unsafe {
            CreateProcessW(
                PCWSTR(executable.as_ptr()),
                Some(PWSTR(command_line.as_mut_ptr())),
                None,
                None,
                false,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                None,
                PCWSTR(current_directory.as_ptr()),
                &raw const startup_info.StartupInfo,
                &raw mut process_info,
            )?;
        }

        let process = OwnedHandle::new(process_info.hProcess);
        let thread = OwnedHandle::new(process_info.hThread);
        drop(thread);
        drop(pseudo_input);
        drop(pseudo_output);

        Ok(Self {
            pseudo_console,
            input: Some(host_input),
            output: Some(host_output),
            process,
            process_id: process_info.dwProcessId,
            size,
        })
    }

    #[must_use]
    pub const fn process_id(&self) -> u32 {
        self.process_id
    }

    #[must_use]
    pub const fn size(&self) -> PtySize {
        self.size
    }

    /// Resizes the pseudoconsole's character grid.
    ///
    /// # Errors
    ///
    /// Returns an error when `ResizePseudoConsole` fails.
    pub fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        // SAFETY: the HPCON remains owned by self and size has already been validated.
        unsafe { ResizePseudoConsole(self.pseudo_console.0, size.coord())? };
        self.size = size;
        Ok(())
    }

    /// Writes all bytes to the terminal input pipe.
    ///
    /// # Errors
    ///
    /// Returns an error when the pipe write fails or completes without accepting any bytes.
    pub fn write_all(&mut self, mut bytes: &[u8]) -> Result<(), PtyError> {
        let input = self
            .input
            .as_ref()
            .ok_or(PtyError::PipeAlreadyTaken("input"))?;
        while !bytes.is_empty() {
            let mut written = 0;
            // SAFETY: input is a live pipe handle and bytes/written remain valid for the call.
            unsafe { WriteFile(input.raw(), Some(bytes), Some(&raw mut written), None)? };
            if written == 0 {
                return Err(PtyError::WriteZero);
            }
            bytes = &bytes[written as usize..];
        }
        Ok(())
    }

    /// Blocks until terminal output is available and reads it into `buffer`.
    ///
    /// # Errors
    ///
    /// Returns an error when the output pipe read fails.
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, PtyError> {
        let output = self
            .output
            .as_ref()
            .ok_or(PtyError::PipeAlreadyTaken("output"))?;
        let mut read = 0;
        // SAFETY: output is a live pipe handle and buffer/read remain valid for the call.
        unsafe { ReadFile(output.raw(), Some(buffer), Some(&raw mut read), None)? };
        Ok(read as usize)
    }

    /// Transfers the blocking output reader to a dedicated worker.
    ///
    /// # Errors
    ///
    /// Returns [`PtyError::PipeAlreadyTaken`] when called more than once.
    pub fn take_reader(&mut self) -> Result<ConPtyReader, PtyError> {
        self.output
            .take()
            .map(|handle| ConPtyReader { handle })
            .ok_or(PtyError::PipeAlreadyTaken("output"))
    }

    /// Transfers the input writer to a dedicated worker.
    ///
    /// # Errors
    ///
    /// Returns [`PtyError::PipeAlreadyTaken`] when called more than once.
    pub fn take_writer(&mut self) -> Result<ConPtyWriter, PtyError> {
        self.input
            .take()
            .map(|handle| ConPtyWriter { handle })
            .ok_or(PtyError::PipeAlreadyTaken("input"))
    }

    /// Checks whether the root process has exited without blocking.
    ///
    /// # Errors
    ///
    /// Returns an error when waiting for or querying the process fails.
    pub fn try_wait(&self) -> Result<Option<ExitStatus>, PtyError> {
        self.wait_for(Duration::ZERO)
    }

    /// Waits up to `timeout` for the root process to exit.
    ///
    /// # Errors
    ///
    /// Returns an error when waiting for or querying the process fails.
    pub fn wait_for(&self, timeout: Duration) -> Result<Option<ExitStatus>, PtyError> {
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        // SAFETY: process is a live process handle owned by self.
        let wait_result = unsafe { WaitForSingleObject(self.process.raw(), timeout_ms) };
        if wait_result == WAIT_TIMEOUT {
            return Ok(None);
        }
        if wait_result != WAIT_OBJECT_0 {
            return Err(PtyError::UnexpectedWaitResult(wait_result.0));
        }

        let mut code = 0;
        // SAFETY: process is signaled and code points to writable storage.
        unsafe { GetExitCodeProcess(self.process.raw(), &raw mut code)? };
        Ok(Some(ExitStatus { code }))
    }
}

pub struct ConPtyReader {
    handle: OwnedHandle,
}

impl ConPtyReader {
    /// Blocks until terminal output is available and reads it into `buffer`.
    ///
    /// # Errors
    ///
    /// Returns an error when the output pipe read fails.
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, PtyError> {
        let mut read = 0;
        // SAFETY: handle is a live pipe and buffer/read remain valid for the call.
        unsafe { ReadFile(self.handle.raw(), Some(buffer), Some(&raw mut read), None)? };
        Ok(read as usize)
    }
}

pub struct ConPtyWriter {
    handle: OwnedHandle,
}

impl ConPtyWriter {
    /// Writes all bytes to the terminal input pipe.
    ///
    /// # Errors
    ///
    /// Returns an error when the pipe write fails or completes without accepting any bytes.
    pub fn write_all(&mut self, mut bytes: &[u8]) -> Result<(), PtyError> {
        while !bytes.is_empty() {
            let mut written = 0;
            // SAFETY: handle is a live pipe and bytes/written remain valid for the call.
            unsafe { WriteFile(self.handle.raw(), Some(bytes), Some(&raw mut written), None)? };
            if written == 0 {
                return Err(PtyError::WriteZero);
            }
            bytes = &bytes[written as usize..];
        }
        Ok(())
    }
}

struct OwnedHandle(HANDLE);

// Win32 handles can be used from any thread. This wrapper uniquely owns its handle and only
// exposes operations whose buffers are scoped to the calling thread.
unsafe impl Send for OwnedHandle {}

impl OwnedHandle {
    const fn new(handle: HANDLE) -> Self {
        Self(handle)
    }

    const fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: this wrapper uniquely owns the handle and closes it exactly once.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

struct PseudoConsole(HPCON);

impl Drop for PseudoConsole {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns the pseudoconsole and closes it exactly once.
        unsafe { ClosePseudoConsole(self.0) };
    }
}

struct AttributeList {
    storage: Vec<usize>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl AttributeList {
    fn new(attribute_count: u32) -> Result<Self, PtyError> {
        let mut required_bytes = 0;
        // SAFETY: a null first call is the documented size query. Its expected insufficient
        // buffer error is ignored; required_bytes receives the necessary allocation size.
        let _ = unsafe {
            InitializeProcThreadAttributeList(None, attribute_count, None, &raw mut required_bytes)
        };

        let words = required_bytes.div_ceil(size_of::<usize>());
        let mut storage = vec![0_usize; words];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast::<c_void>());
        // SAFETY: storage is pointer-aligned and has at least required_bytes writable bytes.
        unsafe {
            InitializeProcThreadAttributeList(
                Some(list),
                attribute_count,
                None,
                &raw mut required_bytes,
            )?;
        }
        Ok(Self { storage, list })
    }

    const fn raw(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.list
    }

    fn set_pseudo_console(&mut self, pseudo_console: HPCON) -> Result<(), PtyError> {
        // PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE is unusual: lpValue is the opaque HPCON value
        // itself, not a pointer to a separate HPCON variable.
        unsafe {
            UpdateProcThreadAttribute(
                self.list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                Some(pseudo_console.0 as *const c_void),
                size_of::<HPCON>(),
                None,
                None,
            )?;
        }
        Ok(())
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // Keep an explicit read so the backing allocation is considered used and, importantly,
        // remains alive until after DeleteProcThreadAttributeList returns.
        let _ = self.storage.len();
        // SAFETY: list was initialized successfully and is deleted exactly once.
        unsafe { DeleteProcThreadAttributeList(self.list) };
    }
}

fn create_pipe() -> Result<(OwnedHandle, OwnedHandle), PtyError> {
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    // SAFETY: both pointers refer to writable HANDLE storage. ConPTY requires synchronous pipes.
    unsafe { CreatePipe(&raw mut read, &raw mut write, None, 0)? };
    Ok((OwnedHandle::new(read), OwnedHandle::new(write)))
}

fn to_wide_null(value: &OsStr) -> Result<Vec<u16>, PtyError> {
    let mut wide: Vec<u16> = value.encode_wide().collect();
    if wide.contains(&0) {
        return Err(PtyError::EmbeddedNull);
    }
    wide.push(0);
    Ok(wide)
}

fn build_command_line(executable: &Path, arguments: &[String]) -> Result<Vec<u16>, PtyError> {
    let mut command_line = quote_windows_argument(&executable.as_os_str().to_string_lossy());
    for argument in arguments {
        command_line.push(' ');
        command_line.push_str(&quote_windows_argument(argument));
    }
    to_wide_null(OsStr::new(&command_line))
}

fn quote_windows_argument(argument: &str) -> String {
    if !argument.is_empty()
        && !argument
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return argument.to_owned();
    }

    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_quoting_handles_spaces_quotes_and_trailing_slashes() {
        assert_eq!(quote_windows_argument("plain"), "plain");
        assert_eq!(quote_windows_argument("two words"), "\"two words\"");
        assert_eq!(quote_windows_argument("a\"b"), "\"a\\\"b\"");
        assert_eq!(
            quote_windows_argument("path with space\\"),
            "\"path with space\\\\\""
        );
    }

    #[test]
    fn conpty_emits_output_and_resizes() {
        let command_processor = std::env::var_os("COMSPEC").map_or_else(
            || PathBuf::from(r"C:\Windows\System32\cmd.exe"),
            PathBuf::from,
        );
        let current_directory = std::env::current_dir().unwrap();
        let command = SpawnCommand::new(command_processor, current_directory).arguments([
            "/d",
            "/c",
            "echo",
            "ADE_CONPTY_OK",
        ]);
        let mut session = ConPtySession::spawn(&command, PtySize::new(80, 24).unwrap()).unwrap();

        session.resize(PtySize::new(100, 30).unwrap()).unwrap();
        assert_eq!(session.size(), PtySize::new(100, 30).unwrap());
        let mut output = Vec::new();
        let mut buffer = [0_u8; 4096];
        while !output
            .windows(b"ADE_CONPTY_OK".len())
            .any(|window| window == b"ADE_CONPTY_OK")
        {
            let bytes_read = session.read(&mut buffer).unwrap();
            assert!(bytes_read > 0, "ConPTY closed before emitting the marker");
            output.extend_from_slice(&buffer[..bytes_read]);
        }

        let status = session
            .wait_for(Duration::from_secs(5))
            .unwrap()
            .expect("cmd.exe did not exit");
        assert_eq!(
            status.code,
            0,
            "output: {}",
            String::from_utf8_lossy(&output)
        );
    }

    #[test]
    fn conpty_delivers_input_to_an_interactive_shell() {
        let command_processor = std::env::var_os("COMSPEC").map_or_else(
            || PathBuf::from(r"C:\Windows\System32\cmd.exe"),
            PathBuf::from,
        );
        let current_directory = std::env::current_dir().unwrap();
        let command =
            SpawnCommand::new(command_processor, current_directory).arguments(["/d", "/q"]);
        let mut session = ConPtySession::spawn(&command, PtySize::new(80, 24).unwrap()).unwrap();

        session.write_all(b"echo ADE_INPUT_OK & exit\r").unwrap();

        let mut output = Vec::new();
        let mut buffer = [0_u8; 4096];
        while !output
            .windows(b"ADE_INPUT_OK".len())
            .any(|window| window == b"ADE_INPUT_OK")
        {
            let bytes_read = session.read(&mut buffer).unwrap();
            assert!(bytes_read > 0, "ConPTY closed before emitting the marker");
            output.extend_from_slice(&buffer[..bytes_read]);
        }

        let status = session
            .wait_for(Duration::from_secs(5))
            .unwrap()
            .expect("cmd.exe did not exit after receiving input");
        assert_eq!(
            status.code,
            0,
            "output: {}",
            String::from_utf8_lossy(&output)
        );
    }

    #[test]
    fn child_exit_does_not_end_root_powershell() {
        let powershell = PathBuf::from(std::env::var_os("WINDIR").unwrap())
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let current_directory = std::env::current_dir().unwrap();
        let command = SpawnCommand::new(powershell, current_directory).arguments([
            "-NoLogo",
            "-NoProfile",
            "-NoExit",
            "-Command",
            "cmd.exe /d /c exit 0; Write-Output ADE_CHILD_EXIT_OK",
        ]);
        let mut session = ConPtySession::spawn(&command, PtySize::new(120, 40).unwrap()).unwrap();
        let mut output = Vec::new();
        let mut buffer = [0_u8; 4096];
        while !output
            .windows(b"ADE_CHILD_EXIT_OK".len())
            .any(|window| window == b"ADE_CHILD_EXIT_OK")
        {
            let read = session.read(&mut buffer).unwrap();
            assert!(read > 0, "ConPTY closed before emitting the marker");
            output.extend_from_slice(&buffer[..read]);
        }

        assert_eq!(session.try_wait().unwrap(), None);
        session.write_all(b"exit\r").unwrap();
        assert!(session.wait_for(Duration::from_secs(5)).unwrap().is_some());
    }
}
