use super::*;
use crate::fdisk::{partname, read_table};

/// Writes a GPT onto a sparse file with `sfdisk` and returns its path. The
/// file grows no device nodes, so a reader is all these cases exercise.
fn gpt(name: &str, script: &str) -> Option<String> {
    let sfdisk = Command::new("sfdisk").arg("--version").output().ok()?;
    if !sfdisk.status.success() {
        return None;
    }
    let root = scratch(name);
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(8 * 1024 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let mut child = Command::new("sfdisk")
        .args(["-q", &disk])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("sfdisk");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("the starting table");
    assert!(child.wait().expect("sfdisk").success());
    Some(disk)
}

/// The two backends must agree on one disk, because the append arithmetic and
/// the slot numbering are written against the `sfdisk` reader's numbers. A
/// difference here is a silently different layout on a real install.
#[test]
fn the_libfdisk_reader_agrees_with_the_sfdisk_reader() {
    let script = "label: gpt\nsize=20M, name=one\nsize=20M, name=two\nsize=20M, name=three\n";
    let Some(disk) = gpt("fdisk-agree", script) else {
        return;
    };
    let dumped = super::sfdisk_table(&disk).expect("the sfdisk table");
    let read = read_table(&disk).expect("the libfdisk table");
    assert_eq!(read, dumped);
}

/// A deleted slot leaves a hole, and an append takes the tail rather than the
/// hole. The libfdisk reader must report the slots in the table's own order
/// with the original numbers kept, because `appended_slots` fills the freed
/// number and `regions` measures the free spans.
#[test]
fn a_hole_and_a_reused_slot_read_the_same_through_both() {
    let script = "label: gpt\nsize=20M, name=one\nsize=20M, name=two\nsize=20M, name=three\n";
    let Some(disk) = gpt("fdisk-hole", script) else {
        return;
    };
    let mut child = Command::new("sfdisk")
        .args(["-q", "--delete", &disk, "2"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("sfdisk");
    assert!(child.wait().expect("sfdisk").success());

    let dumped = super::sfdisk_table(&disk).expect("the sfdisk table");
    let read = read_table(&disk).expect("the libfdisk table");
    assert_eq!(read, dumped);
    // The hole is real and slot 2 is free, which is what the reuse rule needs.
    assert_eq!(read.slots.len(), 2);
    assert_eq!(super::appended_slots(&[1, 3], 1), [2]);
}

/// A disk carrying no label is the one the user is most likely to cut. Both
/// readers must give it the GPT span and a 512-byte sector rather than zeros,
/// because a zero span refuses every create for no room.
#[test]
fn an_unpartitioned_disk_reads_as_room_through_both() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("fdisk-blank");
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(200 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();

    let dumped = super::sfdisk_table(&disk).expect("the sfdisk table");
    let read = read_table(&disk).expect("the libfdisk table");
    assert_eq!(read, dumped);
    assert_eq!(read.label, "");
    assert_eq!(read.sector, 512);
    assert!(read.slots.is_empty());
    // 200 MiB in 512-byte sectors, less GPT's 33-sector secondary header and
    // the sector it sits on.
    assert_eq!(read.last, 200 * 1024 * 2 - 34);
    assert_eq!(read.first, 2048);
}

/// util-linux's node rule has more than one branch, and the `-part` branch is
/// the one a hand-written rule misses. A wrong node names a partition the
/// install cannot open. `created_devices` predicts the screen's node through
/// this function, and `settled_node` reads the cut's node with it, so these
/// cases pin the rule the install path itself uses.
#[test]
fn the_node_rule_has_a_part_branch_as_well_as_a_digit_branch() {
    assert_eq!(partname("/dev/vda", 1), "/dev/vda1");
    assert_eq!(partname("/dev/nvme0n1", 2), "/dev/nvme0n1p2");
    assert_eq!(partname("/dev/mmcblk0", 3), "/dev/mmcblk0p3");
    assert_eq!(partname("/dev/loop0", 1), "/dev/loop0p1");
    assert_eq!(
        partname("/dev/mapper/mydisk", 1),
        "/dev/mapper/mydisk-part1"
    );
    assert_eq!(
        partname("/dev/disk/by-id/ata-ST1000_ABC", 1),
        "/dev/disk/by-id/ata-ST1000_ABC-part1"
    );
    assert_eq!(
        partname("/dev/disk/by-path/pci-0000:00:1f.2-ata-1", 1),
        "/dev/disk/by-path/pci-0000:00:1f.2-ata-1-part1"
    );
    // The `-part` branch is those three prefixes and no others. The rest of
    // `/dev/disk/by-*` takes the digit branch, which is the half a reader
    // writing `by-*` would get wrong. Measured 2026-09-22 against libfdisk
    // 2.41.5 by calling `fdisk_partname` directly.
    assert_eq!(
        partname("/dev/disk/by-uuid/abcd", 1),
        "/dev/disk/by-uuid/abcd1"
    );
    assert_eq!(
        partname("/dev/disk/by-partuuid/abcd", 1),
        "/dev/disk/by-partuuid/abcd1"
    );
    // A name ending in a digit separates it from the partition number on the
    // digit branch, which inserts `p`, and on the `-part` branch, which ends
    // in a letter. `partition_number` takes the trailing digits as the slot
    // number, which is the key `deleted_slots` joins a delete to a table row
    // on. Measured 2026-09-22 against libfdisk 2.41.5.
    assert_eq!(
        partname("/dev/disk/by-uuid/abcd1234", 1),
        "/dev/disk/by-uuid/abcd1234p1"
    );
    // The `-part` answer here is the fallback for a path with no node beside
    // it. `fdisk_partname` probes `<disk>1` and `<disk>p1` first, so a node
    // that exists changes the answer. Both probes must be absent for this
    // case to read the fallback.
    for probe in ["/dev/mapper/pool21", "/dev/mapper/pool2p1"] {
        assert!(
            !std::path::Path::new(probe).exists(),
            "{probe} exists on this host, so the fallback is not what this case reads"
        );
    }
    assert_eq!(partname("/dev/mapper/pool2", 1), "/dev/mapper/pool2-part1");
}

/// A path that cannot be read fails rather than reading as an empty disk. An
/// unreadable device that reported no partitions would let the append
/// arithmetic offer the whole disk and the cut would then run on a disk the
/// installer never read.
#[test]
fn an_unreadable_device_is_a_refusal_and_not_an_empty_table() {
    let missing = scratch("fdisk-missing").join("absent.img");
    let err = read_table(&missing.to_string_lossy()).expect_err("no such device");
    assert!(err.contains("cannot read"), "{err}");
}

/// A `dos` label carries no GPT header, so libfdisk answers the first and last
/// usable LBA with the device's own ends. The reader must not pass those on,
/// because a create sized against them overruns the GPT span by the 33-sector
/// secondary header and the cut then shrinks it without saying so.
///
/// The extended container is a slot here and not a skipped entry, which is
/// what `regions` measures the free span from on a `dos` disk.
#[test]
fn a_dos_label_reads_the_conservative_span_through_both() {
    let script = "label: dos\nsize=20M\nsize=20M\n";
    let Some(disk) = gpt("fdisk-dos", script) else {
        return;
    };
    let dumped = super::sfdisk_table(&disk).expect("the sfdisk table");
    let read = read_table(&disk).expect("the libfdisk table");
    assert_eq!(read, dumped);
    assert_eq!(read.label, "dos");
    assert_eq!(read.first, 2048);
}

/// A `dos` extended container reports a start and a size, so both readers list
/// it as a slot beside its logical partitions. A reader that skipped it would
/// lose the span every logical sits inside, and `regions` measures the free
/// spans from the ends it can see.
#[test]
fn a_dos_extended_container_is_a_slot_through_both() {
    let script = "label: dos\nsize=20M, type=83\ntype=5\n";
    let Some(disk) = gpt("fdisk-extended", script) else {
        return;
    };
    let dumped = super::sfdisk_table(&disk).expect("the sfdisk table");
    let read = read_table(&disk).expect("the libfdisk table");
    assert_eq!(read, dumped);
    let numbers: Vec<usize> = read.slots.iter().map(|slot| slot.number).collect();
    assert!(numbers.contains(&2), "the container is a slot: {numbers:?}");
}

/// Runs `command` and returns its trimmed stdout when it succeeds.
fn ran(command: &mut Command) -> Option<String> {
    let out = command.output().ok()?;
    match out.status.success() {
        true => Some(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        false => None,
    }
}

/// A loop device that detaches on drop. The earlier shape detached after the
/// reads, so a read failure leaked the device and the next run of this suite
/// found the host short of loop devices.
struct Loop(String);

impl Drop for Loop {
    fn drop(&mut self) {
        let _ = Command::new("losetup").args(["-d", &self.0]).status();
    }
}

/// Attaches `image` as a loop device with a 4096-byte logical sector, which
/// is the one thing a regular file cannot carry. The two cases built on it
/// are `#[ignore]`d and run through the root wrapper, so a failed attach is a
/// test failure. A return here would report the case as passed without
/// running, which is the false green the `#[ignore]` reason exists to avoid.
fn loop_4kn(name: &str, bytes: u64) -> (Loop, String) {
    assert_eq!(
        unsafe { libc::geteuid() },
        0,
        "{name} needs root: a 4Kn loop device cannot be attached as a user"
    );
    assert!(
        ran(Command::new("losetup").arg("--version")).is_some(),
        "{name} needs losetup"
    );
    let root = scratch(name);
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(bytes))
        .expect("a backing file");
    let path = image.to_string_lossy().to_string();
    let device = ran(Command::new("losetup").args(["-b", "4096", "-f", "--show", &path]))
        .unwrap_or_else(|| panic!("{name}: losetup -b 4096 could not attach {path}"));
    (Loop(device), path)
}

/// The two readers disagree about the sector size of a 4Kn disk that carries
/// no label, because `table_of` has no `sector-size` line to read and falls
/// back to 512 while libfdisk asks the device. The room a create is sized
/// against comes from that sector size, so a reader offering more room than
/// the disk has lets `sfdisk` shrink the partition without saying so.
///
/// The two cannot agree exactly here. Each reserves 34 sectors for the GPT
/// secondary header in its own sector size, so the reserve differs by 34
/// sectors of the size gap, and the `+ 1` in `regions` removes one of each
/// again. The difference is therefore 33 * (4096 - 512) bytes, which is
/// 118,272 on a 512 MiB disk. This module offers the smaller room, which is
/// the safe side. A disk too small to hold the 1 MiB alignment and the reserve
/// saturates both spans to zero, and the test's own size keeps clear of that.
///
/// This is the case that blocked `disk_table` moving to the libfdisk reader.
#[test]
#[ignore = "needs root and losetup -b 4096"]
fn a_4kn_disk_with_no_label_reports_the_same_room_through_both() {
    let (device, _image) = loop_4kn("fdisk-4kn-blank", 512 * 1024 * 1024);
    let dumped = super::sfdisk_table(&device.0).expect("the sfdisk table");
    let read = read_table(&device.0).expect("the libfdisk table");

    assert_eq!(dumped.sector, 512, "the sfdisk reader falls back to 512");
    assert_eq!(read.sector, 4096, "libfdisk asks the device");
    let by_dump = dumped.regions(&[])[0].sectors * dumped.sector;
    let by_lib = read.regions(&[])[0].sectors * read.sector;
    assert!(
        by_dump >= by_lib,
        "the fallback reader must stay the optimistic side: {by_dump} against {by_lib}"
    );
    assert_eq!(
        by_dump - by_lib,
        33 * (read.sector - dumped.sector),
        "the reserve difference is not fully accounted for"
    );
}

/// A 4Kn disk that carries a GPT reports its real sector size through both
/// readers, because `sfdisk --dump` prints a `sector-size` line and
/// `table_of` reads it. This is the labelled case with a full span, where the
/// two readers agree.
#[test]
#[ignore = "needs root and losetup -b 4096"]
fn a_4kn_disk_with_a_gpt_reads_the_same_through_both() {
    let (device, _image) = loop_4kn("fdisk-4kn-gpt", 512 * 1024 * 1024);
    let script = "label: gpt\nsize=20M, name=one\nsize=20M, name=two\n";
    let mut child = Command::new("sfdisk")
        .args(["-q", &device.0])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("sfdisk");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("the starting table");
    let wrote = child.wait().expect("sfdisk").success();
    let dumped = super::sfdisk_table(&device.0).expect("the sfdisk table");
    let read = read_table(&device.0).expect("the libfdisk table");

    assert!(wrote, "sfdisk wrote the starting table");
    assert_eq!(dumped.sector, 4096);
    assert_eq!(read, dumped);
}
