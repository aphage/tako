#![cfg(unix)]

use std::io;
use std::path::Path;
use std::time::Duration;
use std::{fs, os::unix::fs::PermissionsExt};

use tokio::net::{UnixListener, UnixStream};

const CONNECT_RETRY_ATTEMPTS: usize = 50;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(10);

pub fn bind_listener(path: &Path) -> io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    if path.exists() {
        fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub async fn connect_stream(path: &Path) -> io::Result<UnixStream> {
    for attempt in 0..CONNECT_RETRY_ATTEMPTS {
        match UnixStream::connect(path).await {
            Ok(stream) => return Ok(stream),
            Err(err)
                if err.kind() == io::ErrorKind::NotFound
                    || err.kind() == io::ErrorKind::ConnectionRefused =>
            {
                if attempt + 1 == CONNECT_RETRY_ATTEMPTS {
                    return Err(err);
                }
                tokio::time::sleep(CONNECT_RETRY_DELAY).await;
            }
            Err(err) => return Err(err),
        }
    }

    Err(io::Error::other("unix socket client retry loop exhausted"))
}

pub fn cleanup_socket_file(path: &Path) -> io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
