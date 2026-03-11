use tako_ipc::{CallOptions, IpcAddress};

#[test]
fn named_pipe_address_is_normalized() {
    let addr = IpcAddress::NamedPipe("demo".into());
    assert_eq!(addr.normalize(), String::from(r"\\.\pipe\demo"));
}

#[test]
fn call_options_default_to_empty() {
    let options = CallOptions::default();
    assert!(options.timeout.is_none());
    assert!(options.trace_id.is_none());
}

