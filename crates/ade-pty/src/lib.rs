#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{
    ConPtyReader, ConPtySession, ConPtyWriter, ExitStatus, PtyError, PtySize, SpawnCommand,
};

#[cfg(not(windows))]
compile_error!("ade-pty currently supports Windows only");
