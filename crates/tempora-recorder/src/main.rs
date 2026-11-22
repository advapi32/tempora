#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod host;

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use std::process::ExitCode;
    use std::sync::mpsc::{RecvTimeoutError, channel};
    use std::time::Duration;

    if !std::env::args_os().any(|arg| arg == "--quit") {
        host::run();
        return ExitCode::SUCCESS;
    }

    // A stalled pipe peer must not make the installer wait indefinitely.
    let (send, receive) = channel();
    if std::thread::Builder::new()
        .spawn(move || {
            let _ = send.send(quit());
        })
        .is_err()
    {
        return ExitCode::from(2);
    }
    match receive.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Err(RecvTimeoutError::Timeout) => ExitCode::from(1),
        _ => ExitCode::from(2),
    }
}

#[cfg(windows)]
/// Asks a running recorder to quit, then waits until it has released the database.
fn quit() -> anyhow::Result<()> {
    use std::{fs::OpenOptions, io, os::windows::fs::OpenOptionsExt, time::Duration};
    use tempora::{
        model::{ControlRequest, database_path},
        pipe::{self, RequestError},
    };
    use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;

    let lock = database_path()?.with_extension("writer.lock");
    let mut requested = false;
    loop {
        if !requested {
            match pipe::command(&ControlRequest::Quit) {
                Ok(()) => requested = true,
                // Not listening (yet or anymore): the lock below tells which.
                Err(RequestError::NotRunning) => {}
                Err(error) => return Err(error.into()),
            }
        }
        match OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&lock)
        {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) => {}
            Err(error) => return Err(error.into()),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The Tempora recorder requires Windows.");
}
