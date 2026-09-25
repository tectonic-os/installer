//! Drives the installer's screens on a real terminal. Each drawn frame is
//! compared byte for byte against a committed golden.
//!
//! Regenerate the goldens with `UPDATE_GOLDEN=1 cargo test`, then read the
//! diff.

use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tmp() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn empty(name: &str) -> PathBuf {
    let dir = tmp().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn compare(name: &str, file: &str, actual: &str) {
    let actual = actual.replace(env!("CARGO_PKG_VERSION"), "{version}");
    let path = crate_dir().join("tests/golden").join(name).join(file);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{}: {err}\nrun UPDATE_GOLDEN=1 cargo test", path.display()));
    assert!(
        expected == actual,
        "{} changed. Rerun with UPDATE_GOLDEN=1 and read the diff.\n{}",
        path.display(),
        first_difference(&expected, &actual)
    );
}

/// Reports where two goldens first part, as escaped bytes either side of the
/// offset.
///
/// A transcript golden is mostly escape sequences, so a bare equality failure
/// names no bytes at all. A CI runner also discards its checkout when the job
/// ends, so a golden regenerated there never reaches the reader. The
/// difference travels in the failure message instead.
fn first_difference(expected: &str, actual: &str) -> String {
    let at = expected
        .bytes()
        .zip(actual.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or(expected.len().min(actual.len()));
    let window = |s: &str| {
        let from = at.saturating_sub(60);
        let to = (at + 60).min(s.len());
        s.get(from..to)
            .unwrap_or("<not a char boundary>")
            .escape_debug()
            .to_string()
    };
    format!(
        "first difference at byte {at} of {} expected, {} actual\n  expected: {}\n    actual: {}",
        expected.len(),
        actual.len(),
        window(expected),
        window(actual)
    )
}

/// Runs one installer flow on a real terminal and compares the drawn frames
/// against a committed golden. `script` supplies the pty. A reader thread
/// answers every cursor-position query a widget opens with. Each step types
/// after the draw has settled.
///
/// Only the tail from the last `after` is compared, so whatever the pty wrote
/// before the installer's first line stays out of the golden.
fn drawn_flow(name: &str, dir: &Path, command: &str, after: &str, steps: &[&[u8]]) {
    use std::io::{Read, Write};
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let mut child = std::process::Command::new("script")
        .args(["-qfec", command, "/dev/null"])
        .current_dir(dir)
        // The host's own COLUMNS would reach the pty and redraw at that
        // width, so the golden pins the drawn width it captured.
        .env("COLUMNS", "80")
        // A TPM on the host adds the `tpm2-` kinds to the encryption window,
        // so the probe is pointed at a path no machine carries.
        .env("TECT_TPM", "/nonexistent")
        // A caret that redraws on a clock would put frames in the transcript
        // that depend on the runner's timing rather than on the keys typed.
        .env("TECT_CARET", "static")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("script from util-linux");
    let input = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let mut output = child.stdout.take().unwrap();
    let raw = Arc::new(Mutex::new(Vec::new()));
    let reader = {
        let (input, raw) = (input.clone(), raw.clone());
        std::thread::spawn(move || {
            let mut byte = [0];
            while output.read_exact(&mut byte).is_ok() {
                let mut held = raw.lock().unwrap();
                held.push(byte[0]);
                if held.ends_with(b"\x1b[6n") {
                    let mut input = input.lock().unwrap();
                    let _ = input.write_all(b"\x1b[1;1R");
                    let _ = input.flush();
                }
            }
        })
    };
    for keys in steps {
        std::thread::sleep(Duration::from_millis(400));
        let mut input = input.lock().unwrap();
        input.write_all(keys).unwrap();
        input.flush().unwrap();
    }
    // A walk that derails leaves the installer on a screen no step answers.
    // The deadline turns that into a failure with the frames it drew, where an
    // unending wait hides which frame went wrong.
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let status = loop {
        match child.try_wait().unwrap() {
            Some(status) => break status,
            None if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50))
            }
            None => {
                // A kill that fails leaves the wait below with no deadline.
                child
                    .kill()
                    .expect("the installer is killed at the deadline");
                break child.wait().unwrap();
            }
        }
    };
    reader.join().unwrap();
    let mut errors = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut errors)
        .unwrap();
    let raw = raw.lock().unwrap();
    assert!(
        status.success(),
        "{errors}{}",
        String::from_utf8_lossy(&raw)
    );

    let text = String::from_utf8_lossy(&raw);
    let stable = text.rsplit_once(after).unwrap().1;
    compare(
        name,
        "transcript.txt",
        &format!("{after}{stable}==== exit 0\n"),
    );
}

/// `tect-installer.service` owns tty1, but the serial console and the other
/// VTs autologin root with this binary on `PATH`. A second installer is
/// refused there and told which console holds the first. The lock stops two
/// installers partitioning one disk.
///
/// The first installer is held at its screen on a pty while the second asks.
/// A unit test cannot reach that part. The lock has to still be held while
/// the installer runs, and `let _` in place of `_lock` in `main.rs` would
/// release it before the disk is touched.
#[test]
fn a_second_installer_is_refused_while_the_first_holds_the_screen() {
    use std::time::Duration;

    let dir = empty("flow-install-lock");
    std::fs::write(
        dir.join(installer::RECIPE),
        r#"{
  "image": "ghcr.io/tectonic-os/deb2:latest",
  "targetImgref": "ghcr.io/tectonic-os/deb2:latest",
  "composeFsBackend": true,
  "genericImage": true,
  "bootloader": "grub2",
  "filesystem": "ext4",
  "luksInitramfs": true,
  "hostname": "deb2",
  "user": { "groups": ["sudo"] }
}
"#,
    )
    .unwrap();
    let lock = dir.join("installer.lock");
    let second = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_tect-installer"))
            .args(["--from", "."])
            .current_dir(&dir)
            .env("TECT_INSTALLER_LOCK", &lock)
            .env("TECT_TPM", "/nonexistent")
            .output()
            .unwrap()
    };

    // The first installer runs on a pty, so it reaches its screen and waits.
    let mut first = std::process::Command::new("script")
        .args([
            "-qfec",
            &format!(
                "TECT_INSTALLER_LOCK='{}' '{}' --from .",
                lock.display(),
                env!("CARGO_BIN_EXE_tect-installer")
            ),
            "/dev/null",
        ])
        .current_dir(&dir)
        .env("COLUMNS", "80")
        .env("TECT_TPM", "/nonexistent")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("script from util-linux");

    let mut refused = None;
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(50));
        let out = second();
        let said = String::from_utf8_lossy(&out.stderr).into_owned();
        if said.contains("the installer is running on") {
            refused = Some(said);
            break;
        }
    }
    let killed = first.kill().and_then(|()| first.wait());
    let refused = refused.expect("the first installer never held the lock while it ran");
    killed.expect("the first installer is reaped");

    assert!(
        refused.contains(&format!("run `{}` again", installer::PROGRAM)),
        "{refused}"
    );
    // The next console takes the lock once the holder exits. A record lock
    // belongs to the process, so a holder that died leaves nothing to clean
    // up.
    let after = second();
    assert!(
        !String::from_utf8_lossy(&after.stderr).contains("the installer is running on"),
        "the lock outlived the process holding it: {}",
        String::from_utf8_lossy(&after.stderr)
    );
}

/// Walks the installer's one screen over a payload root, on a real terminal.
/// The screen carries every question at once and each one is answered in
/// place. `Install` stays dim until every required answer is given.
///
/// The steps walk that screen and then take `Install`, which draws the
/// summary and the confirmation that costs a disk. The walk leaves from the
/// summary, so no install runs and no disk is written.
#[test]
fn install_screens() {
    use std::os::unix::fs::PermissionsExt;

    let dir = empty("flow-install-drawn");
    std::fs::write(
        dir.join(installer::RECIPE),
        r#"{
  "image": "ghcr.io/tectonic-os/deb2:latest",
  "targetImgref": "ghcr.io/tectonic-os/deb2:latest",
  "composeFsBackend": true,
  "genericImage": true,
  "bootloader": "grub2",
  "filesystem": "ext4",
  "luksInitramfs": true,
  "hostname": "deb2",
  "user": { "groups": ["sudo"] }
}
"#,
    )
    .unwrap();
    // The disk rows the form offers come from the running machine, and no two
    // machines carry the same disks. The golden brings its own `/sys/block`
    // so the drawn rows are the fixture's.
    let sys = dir.join("sys-block");
    for (name, size, removable, model) in [
        ("vda", "134217728", "0", "QEMU HARDDISK"),
        ("sdb", "31457280", "1", "Cruzer Blade"),
    ] {
        let disk = sys.join(name);
        std::fs::create_dir_all(disk.join("device")).unwrap();
        std::fs::write(disk.join("size"), size).unwrap();
        std::fs::write(disk.join("removable"), removable).unwrap();
        std::fs::write(disk.join("device/model"), model).unwrap();
    }
    let lsblk = dir.join("lsblk");
    std::fs::write(
        &lsblk,
        r#"#!/bin/sh
case "$*" in
*--json*"/dev/sdb"*) printf '%s\n' '{"blockdevices":[{"name":"/dev/sdb","type":"disk","children":[]}]}' ;;
*--json*) printf '%s\n' '{"blockdevices":[{"name":"/dev/vda","type":"disk","children":[{"name":"/dev/vda1","size":"512M","fstype":"vfat","label":"EFI","type":"part","parttype":"C12A7328-F81F-11D2-BA4B-00A0C93EC93B"},{"name":"/dev/vda2","size":"63.5G","fstype":"ext4","label":"old-root","type":"part","parttype":null},{"name":"/dev/vda3","size":"60G","fstype":"crypto_LUKS","label":"","type":"part","parttype":null}]}]}' ;;
*SIZE*) printf '%s\n' '64G' ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&lsblk, std::fs::Permissions::from_mode(0o755)).unwrap();
    // The manual layout reads the chosen disk's table through `libfdisk`,
    // which opens the device. The fixture's disks have no node on this rig,
    // so the fixture writes a real GPT onto a sparse file and `TECT_DEV` aims
    // the installer's device reads at the directory holding it. The reader it
    // replaced ran `sfdisk --dump`, which the fixture answered on `PATH`.
    //
    // `sfdisk` is required and not skipped. It ships with the `script` above
    // in util-linux, and a silent return would report the whole drawn flow as
    // passed without drawing it.
    let dev = dir.join("dev");
    std::fs::create_dir_all(&dev).unwrap();
    let image = dev.join("vda");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(64 * 1024 * 1024 * 1024))
        .unwrap();
    let disk = image.to_string_lossy().to_string();
    let mut child = std::process::Command::new("sfdisk")
        .args(["-q", &disk])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("sfdisk from util-linux writes the fixture table");
    {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"label: gpt\nsize=512M, type=U\nsize=10G\nsize=10G\n")
            .unwrap();
    }
    assert!(child.wait().unwrap().success());
    // The manual layout offers every disk the scan read, and picking one of
    // the others switches the plan to it. The other disk therefore carries a
    // table too, or the switch would stop on a read the rig cannot make.
    let other = dev.join("sdb");
    std::fs::File::create(&other)
        .and_then(|file| file.set_len(16 * 1024 * 1024 * 1024))
        .unwrap();
    let other_disk = other.to_string_lossy().to_string();
    let mut child = std::process::Command::new("sfdisk")
        .args(["-q", &other_disk])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("sfdisk from util-linux writes the fixture table");
    {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"label: gpt\n")
            .unwrap();
    }
    assert!(child.wait().unwrap().success());
    // The walk mounts what the disk holds read-only. The disk does not exist
    // on this rig, so the fixture answers `mount` itself: it mounts nothing
    // and writes onto the mount point the few files the walk reads, which is
    // what lets the drawn table carry a detected system. A VM proof covers
    // what the walk does with a real disk.
    let fake_mount = dir.join("mount");
    std::fs::write(
        &fake_mount,
        r#"#!/bin/sh
# `mount_ro` passes the device third and the mount point last.
case "$3" in
/dev/vda1) mkdir -p "$4/EFI/fedora" ;;
/dev/vda2)
    mkdir -p "$4/etc"
    printf 'PRETTY_NAME="Test OS"\n' > "$4/etc/os-release"
    ;;
esac
exit 0
"#,
    )
    .unwrap();
    std::fs::set_permissions(&fake_mount, std::fs::Permissions::from_mode(0o755)).unwrap();
    let fake_umount = dir.join("umount");
    std::fs::write(&fake_umount, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake_umount, std::fs::Permissions::from_mode(0o755)).unwrap();
    // The encryption row reads the container's header with `luksDump`, which
    // never opens the container. The rig has no `/dev/vda3`, so without this
    // fixture the row would draw what the host's `cryptsetup` says about an
    // absent device.
    let cryptsetup = dir.join("cryptsetup");
    std::fs::write(
        &cryptsetup,
        r#"#!/bin/sh
case "$1" in
luksDump) printf '%s\n' '{"keyslots":{"0":{}},"tokens":{}}' ;;
*) exit 1 ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&cryptsetup, std::fs::Permissions::from_mode(0o755)).unwrap();
    // The manual table asks the image which entry directories its staged EFI
    // payloads carry, so the picture draws what replaces the entries it
    // removes. The rig has no payload image, so the fixture answers the list.
    let podman = dir.join("podman");
    std::fs::write(
        &podman,
        r#"#!/bin/sh
case "$*" in
*"/usr/lib/efi"*) printf '%s\n' /usr/lib/efi/grub2/1/EFI/fedora /usr/lib/efi/shim/1/EFI/BOOT ;;
*) exit 1 ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&podman, std::fs::Permissions::from_mode(0o755)).unwrap();
    // The panel states the firmware of the machine running it. That machine
    // may be UEFI or BIOS, and no two agree on their variables, so the golden
    // brings its own efivars.
    let efivars = dir.join("efivars");
    std::fs::create_dir_all(&efivars).unwrap();
    for (name, value) in [("SecureBoot", 0u8), ("SetupMode", 1u8)] {
        let mut bytes = vec![0, 0, 0, 7];
        bytes.push(value);
        std::fs::write(
            efivars.join(format!("{name}-8be4df61-93ca-11d2-aa0d-00e098032b8c")),
            bytes,
        )
        .unwrap();
    }
    // The switch note names the VT the installer sits on. A machine with no
    // console reads nothing here, and a machine with one reads whichever VT
    // the user started the run from. The golden brings its own VT instead.
    let active = dir.join("tty0-active");
    std::fs::write(&active, "tty2\n").unwrap();
    drawn_flow(
        "flow-install-drawn",
        &dir,
        // The installer media's console is 50 rows. A 24-row pty would scroll
        // the answers the walk asserts out of the captured frames, so the pty
        // is pinned to the media console's height.
        &format!(
            "stty rows 50 cols 80; PATH='{}':\"$PATH\" TECT_INSTALLER_LOCK='{}' TECT_SYS_BLOCK='{}' TECT_DEV='{}' TECT_MOUNT_ROOT='{}' TECT_EFIVARS='{}' TECT_TTY0_ACTIVE='{}' '{}' --from .",
            dir.display(),
            dir.join("installer.lock").display(),
            sys.display(),
            dev.display(),
            dir.join("mounts").display(),
            efivars.display(),
            active.display(),
            env!("CARGO_BIN_EXE_tect-installer")
        ),
        // The installer prints the discovery line last before the first
        // widget draws. Anchoring there keeps every question in the golden.
        &format!("{}: ghcr.io/tectonic-os/deb2:latest, from .\r\n", installer::PROGRAM),
        &[
            // `Install` is dim before any answer is given and says nothing
            // on its own. The walk moves down to it and takes it, which draws
            // the missing answers, and then comes back up. Typing enters a
            // field, so the walk changes rows with the arrow keys.
            b"\x1b[B", b"\x1b[B", b"\x1b[B", b"\x1b[B", b"\x1b[B", b"\x1b[B", b"\x1b[B",
            b"\x1b[B",
            b"\r",
            b"\x1b[A", b"\x1b[A", b"\x1b[A", b"\x1b[A", b"\x1b[A", b"\x1b[A", b"\x1b[A",
            b"\x1b[A",
            // The hostname starts blank by the owner's decision 2026-09-24,
            // so the walk supplies one before the username.
            b"deb2\r",
            b"tect\r",
            b"hunter2\r",
            b"hunter2\r",
            // The `partition layout` row opens a list of layouts.
            b"\r", // open the pick
            b"\r", // take whole disk
            b"\x1b[B",
            b"\x1b[B",
            b"\x1b[B", // to the table
            b"\r", // table mode
            b"\x1b[B", // to the internal disk
            b"\r", // the disk confirmation opens
            b"", // settle
            b"\r", // Use this disk
            b"", // settle
            // A composefs target hides the separate-home answer, so the home
            // row keeps the system and the home together and asks no size.
            // The walk leaves the table and walks up to the encryption row.
            b"\x1b[A", // the disk above the chosen one
            b"\x1b[A", // out of the table, to the home row
            b"\x1b[A", // to the encryption row
            // The encryption type is a radio group. Taking `none` owes no
            // passphrase, so the window closes on the answer.
            b"\r", // the type window opens
            b"\r", // takes none and closes the window
            b"", // settle
            // A kind that owes a passphrase keeps the window up. The window
            // asks for the passphrase twice, and enter on the confirmation
            // submits it.
            b"\r", // the type window opens
            b"\x1b[B", // to LUKS with passphrase
            b"\r", // takes LUKS with passphrase
            b"opensesame\r", // the passphrase
            b"opensesamex\r", // a mismatch the window refuses
            b"\x7f", // corrects the confirmation
            b"\r", // the corrected confirmation closes the window
            b"", // settle
            b"\x1b[A", // to partition layout
            b"\r", // open the pick
            b"\x1b[B", // to manual
            b"\r", // takes the manual layout
            b"\x1b[B", // to the table
            b"\r", // table mode, which opens on the first disk
            b"\r", // a manual pick of another disk, which asks nothing
            b"\x1b[B", // to the chosen disk
            b"\r", // a manual pick of the chosen disk, asking nothing
            b"\x1b[B", // the first partition
            b"\r", // Assign opens
            b"\r", // its list
            b"\x1b[B", // the mount point
            b"\r", // take /boot/efi
            b"\x1b[B", // the second partition
            b"\r", // Assign opens
            b"\r", // its list
            b"\x1b[B", // the mount point
            b"\r", // take /
            b"\r", // the row menu
            b"\x1b[B", // Format
            b"\r", // the format window
            b"\r", // take ext4
            b"\x1b[B", // the container partition below
            b"\x1b[B", // out of the table, to the actions
            b"\x1b[C", // Switch to shell
            b"\r", // its screen
            b"\x1b", // Go back, which opens on the table again
            b"\x1b[B", // to the actions
            b"\r", // Install
            b"\x1b[C",
            b"\r", // Go back on the summary
            b"\x1b", // the leave question
            b"\x1b[B",
            b"\x1b[B",
            b"\r", // Quit
        ],
    );
    // The password the walk typed is not in the transcript. A serial console
    // keeps that transcript and a failed install is read back from it, so a
    // secret drawn into a frame would outlive the run.
    let transcript =
        std::fs::read_to_string(crate_dir().join("tests/golden/flow-install-drawn/transcript.txt"))
            .unwrap();
    assert!(!transcript.contains("hunter2"), "{transcript}");
    // The panel was drawn from the fixture rather than from this machine.
    // ratatui writes the cells a frame changed in runs, so some panel lines
    // arrive in fragments. `tests::panel` checks the wording whole. A run that
    // stopped drawing the panel loses these words from the transcript.
    for phrase in [
        "OS Image",
        installer::copy::PANEL_FIRMWARE,
        "bootloader",
        // The fixture firmware is in setup mode, so the secure-boot row
        // states the condition key enrolment needs rather than a plain off.
        // The panel draws labels and values in separate columns, so only the
        // value is asserted whole.
        installer::copy::FIRMWARE_SETUP,
    ] {
        assert!(transcript.contains(phrase), "{phrase} is not on the screen");
    }
    // Taking the dim `Install` drew the missing answers, under the blank row
    // the screen sets them apart with.
    assert!(
        transcript.contains("Missing: hostname, installation disk, username, password"),
        "{transcript}"
    );
    // `Install` was reachable. A green golden cannot show that on its own,
    // because a run that leaves at the end exits 0 either way.
    //
    // These asserts take contiguous text only. ratatui writes the cells a
    // frame changed, so a label overlapping what was under it arrives in
    // fragments with cursor moves between the words. The summary's own
    // `Go back` button is not contiguous, so the three lines below prove the
    // summary instead.
    assert!(
        transcript.contains(installer::copy::INSTALLATION_SUMMARY),
        "{transcript}"
    );
    assert!(transcript.contains(installer::copy::READY), "{transcript}");
    assert!(
        transcript.contains(installer::copy::START_INSTALLATION),
        "{transcript}"
    );
    // The action row carries the escape hatch beside `Install`. The walk
    // opened its screen and answered `Go back`, so this is the only place a
    // handover to the shell would have run.
    assert!(
        transcript.contains(installer::copy::EXIT_SHELL),
        "{transcript}"
    );
    // The switch screen names the machine's own tty and the key that returns
    // to the installer. The note is redrawn cell by cell, so only its tail
    // stays contiguous. The returning key is what the screen exists to say.
    assert!(transcript.contains("Ctrl+Alt"), "{transcript}");
    assert!(
        transcript.contains(installer::copy::GO_BACK),
        "{transcript}"
    );
    // The note under the ready line states the cost of continuing. The note
    // is redrawn cell by cell over what was under it, so only its last word
    // stays contiguous.
    assert!(transcript.contains("erased"), "{transcript}");
    // Both disks were offered under the disk row, and the walk took the one
    // its steps moved to. Device names stay contiguous where their models do
    // not.
    assert!(transcript.contains("/dev/sdb"), "{transcript}");
    assert!(transcript.contains("/dev/vda"), "{transcript}");
    assert!(transcript.contains("/dev/vda1"), "{transcript}");
    // The partition table drew what the walk set. The ESP is assigned at
    // `/boot/efi` and carries no format tick, because the walk only assigned
    // it. The root is formatted `ext4`. The container's `crypto_LUKS` row
    // stays closed, and the key window's own test covers opening one.
    for phrase in [
        "filesystem",
        "format",
        "/boot/efi",
        "ext4",
        // The automatic plan's type column names what the cut writes. The
        // manual rows' fixture carries no GPT type, so `linux` can only come
        // from the plan.
        "linux",
        // The container's row is drawn closed, in the filesystem column the
        // owner's fixed widths sized for exactly this word.
        "luks(closed)",
        // Only a `Format` answer draws the tick on the root. The fixture's
        // `lsblk` already says `ext4`, so the filesystem cell alone would not
        // prove the walk's Format ran.
        installer::copy::FORMAT_TICK,
        // The disk row's size comes from the fixture's `/sys/block`. The
        // partition's size comes from the fixture's `lsblk` answer. Both are
        // drawn in decimal GB to one place, so lsblk's binary `60G` reads as
        // 64.4 GB.
        "68.7 GB",
        "64.4 GB",
    ] {
        assert!(
            transcript.contains(phrase),
            "{phrase} is not drawn: {transcript}"
        );
    }
    // A composefs target cannot mount a separate `/var`, so the home row
    // offers one answer and the size question is never drawn.
    assert!(
        !transcript.contains(installer::copy::DATA_SEPARATE),
        "the hidden separate-home answer is drawn: {transcript}"
    );
    // The systems the walk found draw as children of the partitions that
    // carry them. The ESP's `EFI/fedora` directory and the old root's own
    // `os-release` are the two sources the fixture writes.
    for phrase in ["fedora", "Test OS"] {
        assert!(
            transcript.contains(phrase),
            "{phrase} is not drawn: {transcript}"
        );
    }
    // The encryption window's passphrase is not in the transcript either. The
    // walk typed it twice, and neither the passphrase field nor its
    // confirmation drew the bytes.
    assert!(!transcript.contains("opensesame"), "{transcript}");
    // The manual table opens the format window. These asserts take its title
    // and the list the walk chose from.
    assert!(
        transcript.contains(installer::copy::SELECT_FORMAT),
        "{transcript}"
    );
    assert!(
        transcript.contains("btrfs") && transcript.contains("xfs"),
        "{transcript}"
    );
}
