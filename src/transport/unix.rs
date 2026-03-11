#![cfg(unix)]

use std::io;
use std::path::Path;
use std::{fs, os::unix::fs::PermissionsExt};

use tokio::net::{UnixListener, UnixStream};

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
    UnixStream::connect(path).await
}

pub fn cleanup_socket_file(path: &Path) -> io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
