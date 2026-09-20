use super::*;

/// Records a running installer. `/run` is a tmpfs, so the lock cannot outlive
/// the boot, and an install ends in a reboot.
///
/// `$TECT_INSTALLER_LOCK` overrides the path, for the same reason `$TECT_TPM`
/// and `$TECT_SYS_BLOCK` do. `/run` is root-owned, and a test or a developer
/// who is not root has nowhere to put the lock file.
const LOCK: &str = "/run/tect-installer.lock";

pub(crate) fn lock() -> PathBuf {
    std::env::var_os("TECT_INSTALLER_LOCK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(LOCK))
}

/// Names the terminal a process is sitting on.
pub(crate) fn console(pid: libc::pid_t) -> String {
    name(std::fs::read_link(format!("/proc/{pid}/fd/0")).ok())
}

/// Names a terminal for the user reading the refusal.
///
/// A VT or a serial line names itself, and the user can walk to it. **A pty
/// cannot be named, and this never guesses at one.** kmscon gives its child a
/// pty, and so does sshd, which the Fedora base ships enabled. Naming a pty
/// *the graphical console* would send the user to a screen holding nothing.
///
/// A descriptor that is not a terminal falls back rather than printing
/// `/dev/null` at the user, because the user types this command and a script
/// can redirect it.
pub(crate) fn name(tty: Option<PathBuf>) -> String {
    let is_console = |tty: &Path| {
        tty.parent()
            .is_some_and(|parent| parent.as_os_str() == "/dev")
            && tty
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("tty") || name == "console")
    };
    match tty {
        Some(tty) if is_console(&tty) => tty.display().to_string(),
        Some(tty) if tty.starts_with("/dev/pts/") => {
            format!("another session ({})", tty.display())
        }
        _ => "another console".into(),
    }
}

/// Takes the installer lock. If another installer holds it, this refuses and
/// names the console holding it.
///
/// `tect-installer.service` owns tty1 on installer media, so the autostart
/// cannot start twice. The lock covers the other way in. The serial console
/// and the other VTs autologin root, and this binary is on `PATH` there. The
/// lock stops two installers partitioning one disk. A guard in the unit would
/// not cover a command the user types.
///
/// The lock belongs to the file and never to its contents. `F_GETLK` is
/// answered from the same kernel state that refused the lock, so no window
/// exists where the file is present and says nothing, and no written-down pid
/// goes stale.
///
/// **A record lock belongs to the process, which is why this uses `F_SETLK`
/// and not `flock(2)`.** An `flock` belongs to the open file description, so
/// a child inheriting the descriptor keeps it alive. This command runs podman,
/// and conmon and fuse-overlayfs double-fork and outlive the install, so an
/// `flock` here would be held until reboot with the media stranded behind a
/// holder the user cannot see. `F_SETLK` is not inherited, and `std` opens
/// `O_CLOEXEC` so the descriptor does not travel either.
pub fn hold() -> Result<std::fs::File, String> {
    hold_at(&lock())
}

/// Takes the lock at a path a test can own.
pub(crate) fn hold_at(path: &Path) -> Result<std::fs::File, String> {
    use std::os::fd::AsRawFd as _;

    let file = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|why| {
            // `/run` is root's, and this command installs an operating
            // system, so a lock that cannot be taken refuses rather than
            // installs.
            format!(
                "{}: {why}\nset $TECT_INSTALLER_LOCK to somewhere writable to run this without root",
                path.display()
            )
        })?;
    let wrlck = || libc::flock {
        l_type: libc::F_WRLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    // Two passes, because `F_GETLK` answers `F_UNLCK` when the holder exited
    // between the two calls. The second pass takes the lock the holder freed.
    for _ in 0..2 {
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &wrlck()) } != -1 {
            return Ok(file);
        }
        let denied = std::io::Error::last_os_error().raw_os_error();
        if denied != Some(libc::EACCES) && denied != Some(libc::EAGAIN) {
            return Err(format!(
                "{}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        let mut holder = wrlck();
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut holder) } == -1 {
            break;
        }
        if holder.l_type != libc::F_UNLCK as libc::c_short {
            return Err(format!(
                "the installer is running on {}; run `{PROGRAM}` again once it finishes",
                console(holder.l_pid)
            ));
        }
    }
    Err(format!(
        "the installer is running on another console; run `{PROGRAM}` again once it finishes"
    ))
}
