use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;

use ade_protocol::{
    ClientRequest, PROTOCOL_VERSION, ServerEvent, Versioned, pipe_name, read_frame, write_frame,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "list".to_owned());
    let mut pipe = connect()?;

    match command.as_str() {
        "list" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&get_snapshot(&mut pipe)?)?
            );
            return Ok(());
        }
        "new" => {
            let root = arguments.next().map_or_else(
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                PathBuf::from,
            );
            let name = arguments.next().unwrap_or_else(|| {
                root.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Workspace")
                    .to_owned()
            });
            send(&mut pipe, ClientRequest::CreateWorkspace { name, root })?;
        }
        "exec" => {
            let command = arguments.collect::<Vec<_>>().join(" ");
            if command.is_empty() {
                return Err("exec requires a command".into());
            }
            let snapshot = get_snapshot(&mut pipe)?;
            let workspace_id = snapshot
                .active_workspace_id
                .ok_or("there is no active workspace")?;
            let pane_id = snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.id == workspace_id)
                .map(|workspace| workspace.active_pane_id)
                .ok_or("the active workspace is missing")?;
            send(
                &mut pipe,
                ClientRequest::Input {
                    pane_id,
                    data: format!("{command}\r").into_bytes(),
                },
            )?;
        }
        "shutdown" => send(&mut pipe, ClientRequest::Shutdown)?,
        _ => {
            return Err(format!(
                "unknown command '{command}'; use list, new [path] [name], exec <command>, or shutdown"
            )
            .into());
        }
    }
    Ok(())
}

fn get_snapshot(pipe: &mut File) -> Result<ade_protocol::AppSnapshot, Box<dyn std::error::Error>> {
    send(pipe, ClientRequest::GetSnapshot)?;
    loop {
        let event: Versioned<ServerEvent> = read_frame(pipe)?;
        ensure_version(event.protocol_version)?;
        if let ServerEvent::Attached { snapshot } = event.message {
            return Ok(snapshot);
        }
    }
}

fn connect() -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(pipe_name())
}

fn send(pipe: &mut File, request: ClientRequest) -> Result<(), ade_protocol::FrameError> {
    write_frame(pipe, &Versioned::new(request))
}

fn ensure_version(version: u32) -> Result<(), Box<dyn std::error::Error>> {
    if version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(format!("daemon protocol {version} does not match client {PROTOCOL_VERSION}").into())
    }
}
