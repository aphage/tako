#![cfg(unix)]

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;

use tako_ipc::transport::unix::{bind_listener, cleanup_socket_file};
use uuid::Uuid;

fn temp_socket_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join(format!("tako-ipc-transport-{label}-{}", Uuid::new_v4()))
        .join("ipc.sock")
}

#[test]
fn bind_listener_creates_restricted_parent_and_socket_permissions() -> io::Result<()> {
    let path = temp_socket_path("permissions");
    let parent = path.parent().expect("socket path should have parent").to_path_buf();

    let listener = bind_listener(&path)?;
    drop(listener);

    let parent_mode = fs::metadata(&parent)?.permissions().mode() & 0o777;
    let socket_mode = fs::metadata(&path)?.permissions().mode() & 0o777;

    assert_eq!(parent_mode, 0o700);
    assert_eq!(socket_mode, 0o600);

    cleanup_socket_file(&path)?;
    fs::remove_dir_all(parent)?;

    Ok(())
}
