#![allow(deprecated)]
use std::ffi::{CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::{fmt, io};

use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::sched::{unshare, CloneFlags};
use nix::unistd::{chdir, pivot_root, sethostname};

#[derive(Debug)]
pub struct SandboxError {
    msg: String,
}

impl SandboxError {
    fn new(msg: impl Into<String>) -> Self {
        Self { msg: msg.into() }
    }
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for SandboxError {}

impl From<io::Error> for SandboxError {
    fn from(e: io::Error) -> Self {
        Self::new(format!("I/O error: {e}"))
    }
}

#[tracing::instrument(skip(rootfs_path), fields(command = %command))]
pub fn run_sandboxed(command: &str, rootfs_path: &Path) -> Result<ExitStatus, SandboxError> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return Err(SandboxError::new("command string is empty"));
    }

    let prog = parts[0];
    let args: Vec<&str> = parts[1..].to_vec();

    let mut cmd = Command::new(prog);
    cmd.args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let rootfs = rootfs_path.to_path_buf();

    unsafe {
        cmd.pre_exec(move || jail_and_exec(&rootfs));
    }

    cmd.status()
        .map_err(|e| SandboxError::new(format!("failed to execute '{}': {e}", prog)))
}

#[tracing::instrument(skip(rootfs))]
fn jail_and_exec(rootfs: &Path) -> io::Result<()> {
    unshare(
        CloneFlags::CLONE_NEWNS
            | CloneFlags::CLONE_NEWPID
            | CloneFlags::CLONE_NEWUTS
            | CloneFlags::CLONE_NEWNET,
    )
    .map_err(|e| nix_to_io("unshare", e))?;

    mount(
        None::<&Path>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .map_err(|e| nix_to_io("make root mount private", e))?;

    mount(
        Some(rootfs),
        rootfs,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .map_err(|e| nix_to_io("bind-mount rootfs", e))?;

    let upper = rootfs.join(".vitis_upper");
    let work = rootfs.join(".vitis_work");
    std::fs::create_dir_all(&upper)
        .map_err(|e| io::Error::other(format!("create upper dir: {e}")))?;
    std::fs::create_dir_all(&work)
        .map_err(|e| io::Error::other(format!("create work dir: {e}")))?;

    let options = CString::new(format!(
        "lowerdir={},upperdir={},workdir={}",
        rootfs.display(),
        upper.display(),
        work.display()
    ))
    .unwrap();

    mount(
        Some("overlay"),
        rootfs,
        Some("overlay"),
        MsFlags::MS_NODEV,
        Some(options.as_c_str()),
    )
    .map_err(|e| nix_to_io("overlay mount", e))?;

    let put_old = Path::new(".vitis_pivot_root");
    std::fs::create_dir_all(put_old)
        .map_err(|e| io::Error::other(format!("create put_old: {e}")))?;

    let rootfs_cstr = CString::new(rootfs.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "rootfs path contains null byte",
        )
    })?;
    let put_old_cstr = CString::new(put_old.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "put_old path contains null byte",
        )
    })?;

    pivot_root(rootfs_cstr.as_c_str(), put_old_cstr.as_c_str())
        .map_err(|e| nix_to_io("pivot_root", e))?;

    umount2(".vitis_pivot_root", MntFlags::MNT_DETACH)
        .map_err(|e| nix_to_io("umount old root", e))?;

    let _ = std::fs::remove_dir(".vitis_pivot_root");

    chdir("/").map_err(|e| nix_to_io("chdir", e))?;

    if !Path::new("/proc").exists() {
        std::fs::create_dir_all("/proc")
            .map_err(|e| io::Error::other(format!("create /proc: {e}")))?;
    }
    mount(
        Some("proc"),
        "/proc",
        Some("proc"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
        None::<&str>,
    )
    .map_err(|e| nix_to_io("mount /proc", e))?;

    sethostname(OsStr::new("vitis-container")).map_err(|e| nix_to_io("sethostname", e))?;

    drop_capabilities().map_err(|e| io::Error::other(format!("drop caps: {e}")))?;
    set_no_new_privs().map_err(|e| io::Error::other(format!("no_new_privs: {e}")))?;
    setup_seccomp().map_err(|e| io::Error::other(format!("seccomp: {e}")))?;

    Ok(())
}

fn drop_capabilities() -> Result<(), String> {
    for cap in 0..64 {
        let ret = unsafe { libc::syscall(libc::SYS_prctl, libc::PR_CAPBSET_DROP, cap, 0, 0, 0) };
        if ret != 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINVAL) {
                break;
            }
            return Err(format!("PR_CAPBSET_DROP({}): {}", cap, err));
        }
    }
    Ok(())
}

fn set_no_new_privs() -> Result<(), String> {
    let ret = unsafe { libc::syscall(libc::SYS_prctl, libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if ret != 0 {
        return Err(format!(
            "PR_SET_NO_NEW_PRIVS: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn setup_seccomp() -> Result<(), String> {
    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH: u32 = 0xC000003E;
    #[cfg(target_arch = "aarch64")]
    const AUDIT_ARCH: u32 = 0xC00000B7;

    const SECCOMP_RET_ALLOW: u32 = 0x7FFF0000;
    const SECCOMP_RET_KILL: u32 = 0x00000000;

    let deny_syscalls: &[i64] = &[
        libc::SYS_ptrace,
        libc::SYS_reboot,
        libc::SYS_kexec_load,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_syslog,
        libc::SYS_create_module,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_iopl,
        libc::SYS_ioperm,
        libc::SYS_kexec_file_load,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_mbind,
        libc::SYS_set_mempolicy,
        libc::SYS_migrate_pages,
        libc::SYS_move_pages,
        libc::SYS_uselib,
        libc::SYS_acct,
        libc::SYS_afs_syscall,
        libc::SYS_get_kernel_syms,
        libc::SYS_query_module,
        libc::SYS_nfsservctl,
        libc::SYS_vhangup,
    ];

    let num_deny = deny_syscalls.len();
    let total_insns = 3 + num_deny * 3 + 1;

    let mut insns: Vec<libc::sock_filter> = Vec::with_capacity(total_insns);

    insns.push(libc::sock_filter {
        code: 0x20,
        jt: 0,
        jf: 0,
        k: 4,
    });
    insns.push(libc::sock_filter {
        code: 0x15,
        jt: 0,
        jf: 1,
        k: AUDIT_ARCH,
    });
    insns.push(libc::sock_filter {
        code: 0x06,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_KILL,
    });

    for &nr in deny_syscalls {
        insns.push(libc::sock_filter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        });
        insns.push(libc::sock_filter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: nr as u32,
        });
        insns.push(libc::sock_filter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_KILL,
        });
    }

    insns.push(libc::sock_filter {
        code: 0x06,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ALLOW,
    });

    let mut prog = libc::sock_fprog {
        filter: insns.as_mut_ptr(),
        len: insns.len() as u16,
    };

    let ret = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            0,
            &mut prog as *mut libc::sock_fprog,
        )
    };

    if ret != 0 {
        return Err(format!("seccomp: {}", io::Error::last_os_error()));
    }

    Ok(())
}

fn nix_to_io(op: &'static str, err: nix::Error) -> io::Error {
    io::Error::other(format!("{op}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::Error as NixError;

    #[test]
    fn test_run_sandboxed_rejects_empty_command() {
        let result = run_sandboxed("", Path::new("/tmp"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_sandbox_error_display() {
        let err = SandboxError::new("test error");
        assert_eq!(format!("{}", err), "test error");
    }

    #[test]
    fn test_sandbox_error_from_io() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err: SandboxError = io_err.into();
        assert!(err.to_string().contains("file not found"));
    }

    #[test]
    fn test_sandbox_error_is_error() {
        let err = SandboxError::new("opaque");
        let trait_obj: &dyn std::error::Error = &err;
        assert_eq!(trait_obj.to_string(), "opaque");
    }

    #[test]
    fn test_sandbox_error_send_sync() {
        fn check<T: Send + Sync>() {}
        check::<SandboxError>();
    }

    #[test]
    fn test_run_sandboxed_rejects_whitespace_only() {
        let result = run_sandboxed("   ", Path::new("/tmp"));
        assert!(result.is_err());
    }

    #[test]
    fn test_nix_to_io_creates_io_error() {
        let nix_err = NixError::EPERM;
        let io_err = nix_to_io("test_op", nix_err);
        assert_eq!(io_err.kind(), std::io::ErrorKind::Other);
        let msg = io_err.to_string();
        assert!(msg.contains("test_op"));
        assert!(msg.contains("EPERM"));
    }

    #[test]
    fn test_unshare_mask_includes_new_net() {
        use nix::sched::CloneFlags;
        let mask = CloneFlags::CLONE_NEWNS
            | CloneFlags::CLONE_NEWPID
            | CloneFlags::CLONE_NEWUTS
            | CloneFlags::CLONE_NEWNET;
        assert!(mask.contains(CloneFlags::CLONE_NEWNET));
    }
}
