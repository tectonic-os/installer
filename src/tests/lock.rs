use super::*;

/// Holds the lock for the test below. A record lock belongs to the process,
/// so a second `hold_at` call inside one test process is granted and proves
/// nothing. The contention has to come from a second process.
#[test]
#[ignore]
fn holds_a_lock_for_another_process_to_find() {
    let Ok(path) = std::env::var("TECT_TEST_LOCK") else {
        // `cargo test -- --ignored` runs this holder with no lock path set.
        // The holder then returns instead of sleeping for 30 seconds.
        return;
    };
    let _held = hold_at(Path::new(&path)).expect("the child takes the lock");
    std::fs::write(format!("{path}.taken"), "").expect("the child reports it");
    std::thread::sleep(std::time::Duration::from_secs(30));
}

/// The installer media starts the installer on tty1 and autologins root on
/// the other consoles, where `tect` is on `PATH`. A second installer the user
/// starts there reaches `hold_at`, so the refusal has to name the first one.
#[test]
fn a_second_installer_is_refused_and_told_where_the_first_one_is() {
    let path = std::env::temp_dir().join(format!("tect-lock-{}", std::process::id()));
    let taken = PathBuf::from(format!("{}.taken", path.display()));
    let _ = std::fs::remove_file(&taken);

    let mut child = Command::new(std::env::current_exe().expect("the test binary"))
        .args([
            "--ignored",
            "--exact",
            "tests::lock::holds_a_lock_for_another_process_to_find",
        ])
        .env("TECT_TEST_LOCK", &path)
        .stdout(Stdio::null())
        .spawn()
        .expect("the test binary re-runs itself as the holder");
    for _ in 0..200 {
        if taken.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(taken.exists(), "the holder never took the lock");

    let refused = hold_at(&path).expect_err("a second installer is refused");
    assert!(
        refused.starts_with("the installer is running on "),
        "{refused}"
    );
    assert!(refused.contains("again once it finishes"), "{refused}");
    // Under `cargo test` the holder inherits the suite's stdin, so the
    // console name it reports is not predictable here. The `name` test
    // below covers which name each stdin produces.

    child.kill().expect("the holder is killed");
    child.wait().expect("the holder is reaped");
    // The kernel drops a record lock when the holder dies, so the next
    // console takes it. The installer clears no stale lock file.
    assert!(
        hold_at(&path).is_ok(),
        "the lock outlived the process holding it"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&taken);
}

/// kmscon and sshd both give their child a pty, so `name` reports a
/// `/dev/pts/N` holder as a session and never as the graphical console.
#[test]
fn a_pty_holder_is_named_as_the_graphical_console() {
    assert_eq!(name(Some("/dev/ttyS0".into())), "/dev/ttyS0");
    assert_eq!(name(Some("/dev/tty1".into())), "/dev/tty1");
    assert_eq!(name(Some("/dev/console".into())), "/dev/console");
    assert_eq!(
        name(Some("/dev/pts/0".into())),
        "another session (/dev/pts/0)"
    );
    // A run redirected from `/dev/null` sits on no terminal. The refusal
    // must not send the next user to `/dev/null` as a console.
    assert_eq!(name(Some("/dev/null".into())), "another console");
    assert_eq!(name(Some("/proc/1/fd/0".into())), "another console");
    // A dead holder leaves no /proc entry, and `console` falls back.
    assert_eq!(name(None), "another console");
    assert_eq!(console(-1), "another console");
}
