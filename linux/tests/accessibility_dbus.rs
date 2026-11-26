#![cfg(target_os = "linux")]

#[test]
fn accessibility_worker_can_connect_without_a_tokio_reactor() {
    let directory = tempfile::tempdir().unwrap();
    let address = format!("unix:path={}/missing", directory.path().display());
    let builder = zbus::connection::Builder::address(address.as_str()).unwrap();

    // AccessKit drives zbus on a plain thread using this executor.
    let error = futures_lite::future::block_on(builder.build()).unwrap_err();
    assert!(
        matches!(&error, zbus::Error::InputOutput(io) if io.kind() == std::io::ErrorKind::NotFound),
        "expected the missing socket to be reported, got {error}"
    );
}
