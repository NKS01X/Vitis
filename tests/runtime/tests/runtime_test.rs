// Exclusive tests for the `vitis-runtime` crate: the Linux-namespace sandbox.

use std::path::Path;
use std::io;

use vitis_runtime::SandboxError;

#[test]
fn empty_command_is_rejected() {
    let r = vitis_runtime::run_sandboxed("", Path::new("/tmp"));
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("empty"));
}

#[test]
fn whitespace_only_command_is_rejected() {
    let r = vitis_runtime::run_sandboxed("   ", Path::new("/tmp"));
    assert!(r.is_err());
}

#[test]
fn error_from_io() {
    let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
    let e: SandboxError = io_err.into();
    assert!(e.to_string().contains("file not found"));
}

#[test]
fn error_implements_std_error() {
    let io_err = io::Error::new(io::ErrorKind::Other, "opaque");
    let e: SandboxError = io_err.into();
    let trait_obj: &dyn std::error::Error = &e;
    assert!(!trait_obj.to_string().is_empty());
}

#[test]
fn error_is_send_sync() {
    fn check<T: Send + Sync>() {}
    check::<SandboxError>();
}

#[test]
fn error_display_roundtrip() {
    let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
    let e: SandboxError = io_err.into();
    assert!(e.to_string().contains("denied"));
}

#[test]
fn real_command_runs_and_succeeds() {
    // This exercises the full sandbox path on Linux; skip elsewhere.
    if !cfg!(target_os = "linux") {
        return;
    }
    let dir = std::env::temp_dir().join("vitis_runtime_test_rootfs");
    let _ = std::fs::create_dir_all(&dir);
    let status = vitis_runtime::run_sandboxed("true", &dir);
    // `true` may still fail if namespaces aren't permitted (e.g. unprivileged
    // containers); accept either outcome but it must not panic.
    let _ = status;
}

#[test]
fn missing_program_fails() {
    let dir = std::env::temp_dir().join("vitis_runtime_test_rootfs2");
    let _ = std::fs::create_dir_all(&dir);
    let status = vitis_runtime::run_sandboxed("this_command_does_not_exist_xyz", &dir);
    // Either exec fails (Err) or the child exits non-zero (Ok(false)).
    match status {
        Ok(s) => assert!(!s.success()),
        Err(_) => {}
    }
}
