use super::*;

/// The table reader takes the three keys the append arithmetic needs. It
/// keeps one slot per partition line in the dump's own order. `/dev/vda2`
/// starts after `/dev/vda3`, so a sector-sorted reader would move it.
#[test]
fn the_table_reader_takes_the_sectors_sfdisk_reports() {
    let table = super::table_of(MEASURED_DUMP);
    assert_eq!(table.first, 2048);
    assert_eq!(table.last, 409566);
    assert_eq!(table.sector, 512);
    assert_eq!(table.slots.len(), 3);
    assert_eq!(table.slots[1].node, "/dev/vda2");
    assert_eq!(table.slots[1].number, 2);
    assert_eq!(table.slots[1].start, 124928);
    assert_eq!(table.slots[1].sectors, 20480);
    // An empty dump names no sector size. `table_of` reads that as 512, so
    // a create on a disk with no table is still sized in GB.
    assert_eq!(super::table_of("").sector, 512);
}

/// `appendable` measures the tail after the highest-ending partition. It
/// never offers a hole between two partitions. `/dev/vda2` ends highest at
/// 145407, so 145408..=409566 is the room a create may take. Measured
/// 2026-09-19, `sfdisk --append` itself takes a hole when the hole is the
/// highest region that fits. See `measurements/sfdisk-append.md`. The tail
/// alone is therefore conservative, and `src/table.rs` records why.
#[test]
fn appendable_room_is_the_tail_and_never_a_hole() {
    let table = super::table_of(MEASURED_DUMP);
    assert_eq!(table.appendable(&[]), 409566 + 1 - 145408);
    // Deleting the highest-ending partition gives its room back. The next
    // highest end is `/dev/vda3`'s 124928.
    let without = vec!["/dev/vda2".to_string()];
    assert_eq!(table.appendable(&without), 409566 + 1 - 124928);
    // `/dev/vda1` does not end highest, so deleting it frees its slot and
    // no room.
    let middle = vec!["/dev/vda1".to_string()];
    assert_eq!(table.appendable(&middle), table.appendable(&[]));
    // Clearing the disk gives the whole usable span back.
    let all: Vec<String> = table.slots.iter().map(|slot| slot.node.clone()).collect();
    assert_eq!(table.appendable(&all), 409566 + 1 - 2048);
}

/// `appendable_gb` multiplies the tail sectors by the sector size. The
/// refusal states decimal GB, like every other size the installer draws.
#[test]
fn appendable_gb_counts_in_decimal_gb() {
    let table = super::table_of(MEASURED_DUMP);
    assert_eq!(
        table.appendable_gb(&[]),
        (409566 + 1 - 145408) * 512 / 1_000_000_000
    );
    // The measured disk spans 200 MiB and holds no whole GB. The division
    // floors, so the editor screen offers the user no create at all.
    assert_eq!(table.appendable_gb(&[]), 0);
}

/// `sfdisk --append` takes the lowest free slot number. With slots 1 and 3
/// present and slot 2 deleted, the appended partition became slot 2,
/// measured 2026-09-19 in `measurements/sfdisk-append.md`.
#[test]
fn a_created_partition_takes_the_lowest_free_slot() {
    let table = super::table_of(MEASURED_DUMP);
    let after = |deletes: &[&str]| {
        table.surviving(&deletes.iter().map(|at| at.to_string()).collect::<Vec<_>>())
    };
    assert_eq!(super::appended_slots(&after(&["/dev/vda2"]), 1), vec![2]);
    assert_eq!(super::appended_slots(&after(&[]), 2), vec![4, 5]);
    assert_eq!(
        super::appended_slots(&after(&["/dev/vda1", "/dev/vda3"]), 3),
        vec![1, 3, 4]
    );
    // `apply_cuts` passes no taken slots for a cleared disk, because the
    // fresh GPT label numbers from 1.
    assert_eq!(
        super::appended_slots(&after(&["/dev/vda1", "/dev/vda2", "/dev/vda3"]), 3),
        vec![1, 2, 3]
    );
}

/// The editor screen and the cut must agree on the node a created partition
/// will carry. The screen counts `lsblk`'s partitions. The cut counts
/// `sfdisk`'s table. A drawn node the cut does not produce leaves the
/// recipe naming a device that is not there, so both read one slot rule.
#[test]
fn the_screen_and_the_cut_agree_on_a_created_node() {
    let table = super::table_of(MEASURED_DUMP);
    let parts: Vec<Partition> = ["/dev/vda1", "/dev/vda2", "/dev/vda3"]
        .into_iter()
        .map(|device| Partition {
            device: device.to_string(),
            ..Default::default()
        })
        .collect();
    for deletes in [
        vec![],
        vec!["/dev/vda2".to_string()],
        vec!["/dev/vda1".to_string(), "/dev/vda3".to_string()],
    ] {
        assert_eq!(
            super::drawn_slots(&parts, &deletes),
            table.surviving(&deletes),
            "deletes {deletes:?}"
        );
    }
}

/// The node a created partition will carry is derived on every call. A
/// delete elsewhere in the plan frees a lower slot and moves that node, so
/// the user's answers travel with the plan entry instead of with a name.
/// A disk whose own name ends in a digit takes the kernel's `p` separator.
#[test]
fn a_created_device_follows_the_slot_it_will_get() {
    let table = super::table_of(MEASURED_DUMP);
    assert_eq!(
        super::created_devices("/dev/vda", &table.surviving(&[]), 2),
        ["/dev/vda4", "/dev/vda5"]
    );
    let gone = vec!["/dev/vda1".to_string()];
    assert_eq!(
        super::created_devices("/dev/vda", &table.surviving(&gone), 2),
        ["/dev/vda1", "/dev/vda4"]
    );
    assert_eq!(
        super::created_devices("/dev/nvme0n1", &table.surviving(&gone), 2),
        ["/dev/nvme0n1p1", "/dev/nvme0n1p4"]
    );
}

/// A delete is named by `lsblk` and a table row by `sfdisk`, and on a
/// `/dev/disk/by-id` or `/dev/disk/by-path` disk those are two different
/// strings for one partition. Measured 2026-09-22, `lsblk --paths` on a by-id
/// disk returns `/dev/nvme0n1p1` while `sfdisk --dump` names its rows with
/// `fdisk_partname`, which answers `<disk>-partN`. A `/dev/mapper` disk
/// differs as well, for the reason `deleted_slots` records.
///
/// Comparing those strings made every delete match nothing. The plan then
/// appended after partitions it was about to remove, and the install was
/// refused by `check_created_slots` after `sfdisk` had already cut the disk.
/// All three delete-aware readers are keyed on the slot number instead.
#[test]
fn a_delete_matches_its_slot_when_lsblk_and_sfdisk_name_it_differently() {
    let dump = "label: gpt\n\
                first-lba: 2048\n\
                last-lba: 409566\n\
                sector-size: 512\n\
                /dev/disk/by-id/ata-X-part1 : start=2048, size=20480\n\
                /dev/disk/by-id/ata-X-part2 : start=124928, size=20480\n\
                /dev/disk/by-id/ata-X-part3 : start=22528, size=20480\n";
    let table = super::table_of(dump);
    assert_eq!(table.slots.len(), 3);
    // What the editor holds: `discover::partitions` asked lsblk, which
    // answers kernel names whatever path the user named the disk by.
    let deletes = vec!["/dev/sda3".to_string()];
    assert_eq!(table.surviving(&deletes), [1, 2]);
    // `appendable` filters on the same key and takes its own arm of
    // `cleared_by`, so it needs a partial delete of its own. Slot 2 ends
    // highest at 145408, so removing it is the delete whose room changes.
    // A comparison that matched nothing would measure from 145408 and refuse
    // a create the disk has the room for.
    let highest = vec!["/dev/sda2".to_string()];
    assert_eq!(table.appendable(&highest), 409566 + 1 - 43008);
    assert_ne!(table.appendable(&highest), table.appendable(&[]));
    assert_eq!(
        super::created_devices("/dev/disk/by-id/ata-X", &table.surviving(&deletes), 1),
        ["/dev/disk/by-id/ata-X-part3"]
    );
    // Deleting every partition clears the table, which is what lets the cut
    // write a fresh GPT label instead of appending into what it is removing.
    let all = vec![
        "/dev/sda1".to_string(),
        "/dev/sda2".to_string(),
        "/dev/sda3".to_string(),
    ];
    assert_eq!(table.surviving(&all), [] as [usize; 0]);
    assert_eq!(table.appendable(&all), 409566 + 1 - 2048);
}

/// **The `/dev/mapper` paths here have no map behind them, and that is what
/// this case pins.** `fdisk_partname` answers `<disk><N>` or `<disk>p<N>`
/// when a node of that name exists, and it falls back to `<disk>-part<N>`
/// only when neither probe finds one. A `/dev/disk/by-id` path reaches the
/// same fallback in the real world, because udev names its symlinks
/// `-part<N>`.
///
/// On a live `/dev/mapper` disk a partition that exists has its `kpartx` node
/// and reads back through the matching probe. A partition the install creates
/// has no node at prediction time, so `created_devices` falls back to
/// `-part<N>` while `kpartx` names the real node `<disk>p<N>` for a map name
/// ending in a digit and `<disk><N>` otherwise. The prediction and the table
/// read back then disagree, and the install is refused after the disk is cut.
/// The path is filed in `BACKLOG.md` and carried into block 2, which re-reads
/// the nodes that appear. `ask_disk` returns `--disk` verbatim, so such a path
/// reaches the cut.
///
/// The prediction takes util-linux's whole node rule and not the digit branch
/// alone. A digit-only rule predicts `...p1` for a table that holds
/// `...-part1`, so the hand-written rule it replaced failed on every
/// `/dev/disk/by-id` path.
#[test]
fn a_created_device_with_no_node_beside_it_takes_the_part_branch() {
    // `fdisk_partname` probes for a node beside the disk name before it falls
    // back to `-part`, so the case only reads the fallback when none of these
    // exists. A host that has one fails the assertion for a reason that has
    // nothing to do with the rule.
    for probe in [
        "/dev/mapper/mydisk4",
        "/dev/mapper/mydiskp4",
        "/dev/mapper/mydisk5",
        "/dev/mapper/mydiskp5",
        "/dev/disk/by-id/wwn-0x50004",
        "/dev/disk/by-id/wwn-0x5000p4",
    ] {
        assert!(
            !std::path::Path::new(probe).exists(),
            "{probe} exists on this host, so the fallback is not what this case reads"
        );
    }
    let table = super::table_of(MEASURED_DUMP);
    assert_eq!(
        super::created_devices("/dev/mapper/mydisk", &table.surviving(&[]), 2),
        ["/dev/mapper/mydisk-part4", "/dev/mapper/mydisk-part5"]
    );
    assert_eq!(
        super::created_devices("/dev/disk/by-id/wwn-0x5000", &table.surviving(&[]), 1),
        ["/dev/disk/by-id/wwn-0x5000-part4"]
    );
}

/// A created ESP is cut as an EFI System partition. Firmware finds the ESP
/// by its GPT type GUID. Cutting the ESP as a Linux filesystem finishes the
/// install and leaves a machine that does not boot.
#[test]
fn a_created_esp_is_cut_as_an_efi_partition() {
    assert_eq!(super::created_type("/boot/efi"), "U");
    assert_eq!(super::created_type("/"), "L");
    assert_eq!(super::created_type(""), "L");
}

/// The cut runs against a real `sfdisk`, on a file-backed GPT. `sfdisk`
/// reads and writes a table in a file, so what it wrote is read back here.
/// A delete the cut cannot number refuses the plan while the disk is still
/// whole. The deletes used to be numbered one at a time inside the loop that
/// cuts them, so a bad entry behind a good one left the disk part way through
/// a plan. This asserts the table is byte-identical after the refusal, which
/// is the outcome the guard exists for. A passing refusal on its own would
/// not show that nothing was cut.
#[test]
fn a_delete_the_cut_cannot_number_takes_nothing_off_the_disk() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("cut-unnamed-delete");
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(200 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let script = "label: gpt
size=20M, name=one
size=20M, name=two
";
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
    let before = super::disk_table(&disk).expect("the table");

    // The good delete comes first, so a loop that numbered each entry as it
    // reached it would have cut slot 1 before refusing the second.
    let layout = CustomLayout {
        disk: disk.clone(),
        deletes: vec![format!("{disk}1"), format!("{disk}-spare")],
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
        }],
        ..Default::default()
    };
    let err = super::apply_cuts(&layout).expect_err("a delete naming no number");
    assert!(err.contains("names no partition number"), "{err}");
    assert_eq!(super::disk_table(&disk).expect("the table"), before);

    // Digits that overflow `usize` are the same refusal. While the readers
    // and the cut tested what is numberable differently, this entry was
    // dropped by the screen and accepted by the cut, so slot 1 went first and
    // `sfdisk` refused the plan afterwards.
    let overflowed = CustomLayout {
        deletes: vec![format!("{disk}1"), format!("{disk}99999999999999999999")],
        ..layout.clone()
    };
    let err = super::apply_cuts(&overflowed).expect_err("a delete that overflows");
    assert!(err.contains("names no partition number"), "{err}");
    assert_eq!(super::disk_table(&disk).expect("the table"), before);

    // The cleared branch writes a fresh label rather than deleting in turn,
    // and it never numbered a delete at all. An unnumberable entry beside a
    // complete set of deletes was accepted there and the label was written.
    let cleared = CustomLayout {
        deletes: vec![
            format!("{disk}1"),
            format!("{disk}2"),
            format!("{disk}-spare"),
        ],
        ..layout.clone()
    };
    let err = super::apply_cuts(&cleared).expect_err("a delete naming no number");
    assert!(err.contains("names no partition number"), "{err}");
    assert_eq!(super::disk_table(&disk).expect("the table"), before);
}

/// The deletes, the appends, the slot numbers and the ESP type are all
/// checked against what the editor screen drew. A file grows no device
/// nodes, and `cut_partitions` waits for those separately.
#[test]
fn the_cut_writes_the_table_the_screen_drew() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("cut-table");
    let image = root.join("disk.img");
    // `set_len` leaves the image sparse. `sfdisk` writes the table only, so
    // the 8 GB in between is never touched and never allocated.
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(8 * 1024 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let script = "label: gpt\nsize=20M, name=one\nsize=20M, name=two\nsize=20M, name=three\n";
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

    // The plan removes the middle partition and cuts an ESP and a root.
    // Slot 2 is the one the delete freed, so the screen draws 2 and 4.
    let before = super::disk_table(&disk).expect("the table");
    let layout = CustomLayout {
        disk: disk.clone(),
        deletes: vec![format!("{disk}2")],
        creates: vec![
            Created {
                gb: 1,
                target: "/boot/efi".to_string(),
                fstype: "fat32".to_string(),
                device: String::new(),
            },
            Created {
                gb: 1,
                target: "/".to_string(),
                fstype: "btrfs".to_string(),
                device: String::new(),
            },
        ],
        ..Default::default()
    };
    let drew = super::created_devices(
        &disk,
        &before.surviving(&layout.deletes),
        layout.creates.len(),
    );
    assert_eq!(drew, [format!("{disk}2"), format!("{disk}4")]);

    let cut = super::apply_cuts(&layout).expect("the cut");
    assert_eq!(cut, [2, 4], "the cut must return the slots the screen drew");

    // The assertions below read the table back from the disk, so they check
    // what `sfdisk` wrote.
    let after = super::disk_table(&disk).expect("the table after");
    let numbers: Vec<usize> = {
        let mut numbers: Vec<usize> = after.slots.iter().map(|slot| slot.number).collect();
        numbers.sort_unstable();
        numbers
    };
    assert_eq!(numbers, [1, 2, 3, 4], "slots after the cut: {numbers:?}");
    // The first create re-took the deleted partition's slot, and it is a
    // whole new partition. The original slot 2 held 20M and this one holds
    // a GB.
    let two = after
        .slots
        .iter()
        .find(|slot| slot.number == 2)
        .expect("slot 2");
    assert!(
        two.sectors * after.sector >= 1_000_000_000,
        "slot 2 is {} sectors, not the created GB",
        two.sectors
    );
    // Both creates land after the last surviving partition. Neither lands
    // in the hole the delete left, which is the rule `appendable` sizes by.
    let three = after
        .slots
        .iter()
        .find(|slot| slot.number == 3)
        .expect("slot 3");
    for number in [2, 4] {
        let slot = after
            .slots
            .iter()
            .find(|slot| slot.number == number)
            .expect("a created slot");
        assert!(
            slot.start >= three.start + three.sectors,
            "slot {number} starts at {}, inside or before the surviving table",
            slot.start
        );
    }
    // The dump must carry the EFI System GUID `created_type` promised.
    let dump = Command::new("sfdisk")
        .args(["--dump", &disk])
        .output()
        .expect("a dump");
    let dump = String::from_utf8_lossy(&dump.stdout);
    let esp = dump
        .lines()
        .find(|line| line.starts_with(&format!("{disk}2 ")))
        .expect("the ESP line");
    assert!(
        esp.to_uppercase()
            .contains("C12A7328-F81F-11D2-BA4B-00A0C93EC93B"),
        "{esp}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A size the disk cannot hold stops the install. `sfdisk` does not refuse
/// one. Measured 2026-09-19, it shrinks the partition and exits 0. A root
/// quietly smaller than the user asked for would otherwise reach fisherman
/// on a disk that had already been cut.
#[test]
fn a_partition_sfdisk_had_to_shrink_stops_the_install() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("cut-clamp");
    let image = root.join("small.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(200 * 1024 * 1024))
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
        .write_all(b"label: gpt\n")
        .expect("an empty table");
    assert!(child.wait().expect("sfdisk").success());
    let layout = CustomLayout {
        disk: disk.clone(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
        }],
        ..Default::default()
    };
    let err = super::apply_cuts(&layout).expect_err("a 1 GB partition on a 200 MiB disk");
    assert!(err.contains("did not say so"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_created_slot_missing_from_the_read_back_table_stops_the_install() {
    let disk = "/dev/vda";
    let layout = CustomLayout {
        disk: disk.to_string(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
        }],
        ..Default::default()
    };
    let err = super::check_created_slots(&layout, &[1], &DiskTable::default())
        .expect_err("a missing sfdisk slot must stop the install");
    assert!(err.contains(&copy::table_already_changed(disk)), "{err}");
}

/// A `/dev/mapper` disk names a created partition only after the cut, so the
/// screen predicts the `-part<N>` fallback and the table comes back with the
/// kpartx name. The check keys on the slot number, which both sides agree on,
/// and the size check reads the slot that number names.
#[test]
fn a_created_slot_is_checked_by_number_when_the_node_name_differs() {
    let layout = CustomLayout {
        disk: "/dev/mapper/mydisk".to_string(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
        }],
        ..Default::default()
    };
    let after = DiskTable {
        label: "gpt".to_string(),
        sector: 512,
        slots: vec![Slot {
            node: "/dev/mapper/mydisk2".to_string(),
            number: 2,
            start: 40960,
            sectors: 1_000_000_000 / 512,
        }],
        ..Default::default()
    };
    assert_eq!(super::check_created_slots(&layout, &[2], &after), Ok(()));
    let short = DiskTable {
        slots: vec![Slot {
            node: "/dev/mapper/mydisk2".to_string(),
            number: 2,
            start: 40960,
            sectors: 1_000_000 / 512,
        }],
        ..after.clone()
    };
    assert!(super::check_created_slots(&layout, &[2], &short).is_err());
}

/// A blank disk and a `dos`-labelled disk both report no GPT span. Read
/// literally, each one refuses every create, including on the blank disk
/// the user is likeliest to cut. `disk_table` derives the span from the
/// device instead.
#[test]
fn a_disk_with_no_gpt_span_still_has_room() {
    // This `dos` dump carries no `first-lba:` and no `last-lba:`.
    let dos = super::table_of(
            "label: dos\nunit: sectors\nsector-size: 512\n\n/dev/vda1 : start=2048, size=40960, type=83\n",
        );
    assert_eq!(dos.label, "dos");
    assert_eq!(dos.last, 0, "a dos dump carries no last-lba");
    // `disk_table` fills that span in, and `table_of` does not. The filled
    // table below checks the effect through the arithmetic that reads it.
    let filled = DiskTable {
        first: 2048,
        last: 409566,
        sector: 512,
        label: "dos".to_string(),
        slots: dos.slots.clone(),
    };
    // A `dos` table the plan keeps is refused. Its numbering does not
    // follow the slot rule, measured on an extended partition.
    assert_eq!(
        filled.wrong_label_for(&[], 1).as_deref(),
        Some(copy::custom_not_gpt("dos").as_str())
    );
    // A plan with no create appends nothing. The label rule guards the
    // append numbering alone, so a `dos` disk whose partitions the user
    // only mounts and formats is not refused.
    assert!(filled.wrong_label_for(&[], 0).is_none());
    // Clearing the disk is allowed. That path writes a fresh `label: gpt`,
    // so the disk comes out GPT and is never appended to as `dos`.
    let all = vec!["/dev/vda1".to_string()];
    assert!(filled.wrong_label_for(&all, 1).is_none());
    // Cleared, the room is the whole usable span. It is not measured from
    // a partition the cut will remove.
    assert_eq!(filled.appendable(&all), 409566 + 1 - 2048);
    // A disk with no table carries an empty label, which is not refused.
    let blank = DiskTable {
        first: 2048,
        last: 409566,
        sector: 512,
        ..Default::default()
    };
    assert!(blank.wrong_label_for(&[], 1).is_none());
    assert_eq!(blank.appendable(&[]), 409566 + 1 - 2048);
}

/// The room is weighed over the whole plan. The user can take a delete back
/// after typing a create, which gives that room to the restored partition.
/// A plan that no longer fits must say so before the confirmation, because
/// `sfdisk` shrinks the create silently after the deletes are written.
#[test]
fn a_plan_that_outgrew_its_room_is_refused_before_the_cut() {
    // The table below is a 100 GB disk holding a 10 GB and a 90 GB partition.
    let sector = 512u64;
    let gb = 1_000_000_000 / sector;
    let table = DiskTable {
        first: 2048,
        last: 100 * gb,
        sector,
        label: "gpt".to_string(),
        slots: vec![
            Slot {
                node: "/dev/vda1".to_string(),
                number: 1,
                start: 2048,
                sectors: 10 * gb,
            },
            Slot {
                node: "/dev/vda2".to_string(),
                number: 2,
                start: 2048 + 10 * gb,
                sectors: 90 * gb - 2048,
            },
        ],
    };
    let plan = |deletes: Vec<&str>, creates: Vec<(u64, &str, &str)>| CustomLayout {
        disk: "/dev/vda".to_string(),
        deletes: deletes.iter().map(|at| at.to_string()).collect(),
        creates: creates
            .into_iter()
            .map(|(gb, target, fstype)| Created {
                gb,
                target: target.to_string(),
                fstype: fstype.to_string(),
                device: String::new(),
            })
            .collect(),
        ..Default::default()
    };
    // The plan deletes the 90 GB partition and cuts an 87 GB root beside a
    // 2 GB ESP. The tail is 90 GB less the 1 MiB head alignment, and
    // `appendable_gb` floors that to 89.
    assert_eq!(table.appendable_gb(&["/dev/vda2".to_string()]), 89);
    let ok = plan(
        vec!["/dev/vda2"],
        vec![(2, "/boot/efi", "fat32"), (87, "/", "btrfs")],
    );
    assert_eq!(layout_short_of(&ok, false, Some(&table), 0), None);
    // One GB over the room is refused, so the boundary sits at the room
    // itself.
    let over = plan(
        vec!["/dev/vda2"],
        vec![(2, "/boot/efi", "fat32"), (88, "/", "btrfs")],
    );
    assert_eq!(
        layout_short_of(&over, false, Some(&table), 0).as_deref(),
        Some(copy::custom_too_big(89).as_str())
    );
    // The user takes the delete back. The 87 GB create now has nowhere to
    // go, and the create itself did not change.
    let mut restored = ok.clone();
    restored.deletes.clear();
    // The disk is full again, so no room is left at all.
    // `copy::custom_too_big` has its own wording for a room of 0 GB.
    assert_eq!(
        layout_short_of(&restored, false, Some(&table), 0).as_deref(),
        Some(copy::custom_too_big(0).as_str())
    );
    // Two creates that each fit and together do not are refused.
    let together = plan(
        vec!["/dev/vda1", "/dev/vda2"],
        vec![(60, "/", "btrfs"), (60, "/boot/efi", "fat32")],
    );
    assert!(layout_short_of(&together, false, Some(&table), 0).is_some());
}

/// A blank disk and a `dos` disk are both cut, against a real `sfdisk`. The
/// old refusal admitted this case and the cut could not carry it.
/// `--append` against a disk with no table fails outright with `Failed to
/// add #2 partition: Invalid argument`. Against a `dos` label it succeeds
/// and leaves the ESP an MBR `ef` type that firmware does not find.
/// Clearing writes a fresh `label: gpt`, so both disks come out GPT.
#[test]
fn a_blank_and_a_dos_disk_are_cut_as_gpt() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("cut-fresh-label");
    let creates = vec![
        Created {
            gb: 1,
            target: "/boot/efi".to_string(),
            fstype: "fat32".to_string(),
            device: String::new(),
        },
        Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
        },
    ];
    // The loop cuts two images. One carries no partition table at all, and
    // the other carries a `dos` label and a partition the plan removes.
    for (name, start, deletes) in [
        ("blank.img", "", vec![]),
        ("dos.img", "label: dos\nsize=20M\n", vec![1usize]),
    ] {
        let image = root.join(name);
        std::fs::File::create(&image)
            .and_then(|file| file.set_len(8 * 1024 * 1024 * 1024))
            .expect("a backing file");
        let disk = image.to_string_lossy().to_string();
        if !start.is_empty() {
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
                .write_all(start.as_bytes())
                .expect("the starting table");
            assert!(child.wait().expect("sfdisk").success());
        }
        let layout = CustomLayout {
            disk: disk.clone(),
            deletes: deletes.iter().map(|at| format!("{disk}{at}")).collect(),
            creates: creates.clone(),
            ..Default::default()
        };
        let cut = super::apply_cuts(&layout).unwrap_or_else(|err| panic!("cutting {name}: {err}"));
        assert_eq!(cut, [1, 2], "{name}");
        let after = super::disk_table(&disk).expect("the table after");
        assert_eq!(after.label, "gpt", "{name} came out {}", after.label);
        assert_eq!(after.slots.len(), 2, "{name}");
        // A GPT type GUID here proves the fresh label replaced the `dos` one.
        let dump = Command::new("sfdisk")
            .args(["--dump", &disk])
            .output()
            .expect("a dump");
        let dump = String::from_utf8_lossy(&dump.stdout);
        let esp = dump
            .lines()
            .find(|line| line.starts_with(&format!("{disk}1 ")))
            .unwrap_or_else(|| panic!("no ESP line for {name} in:\n{dump}"));
        assert!(
            esp.to_uppercase()
                .contains("C12A7328-F81F-11D2-BA4B-00A0C93EC93B"),
            "{name}: {esp}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// The derived span counts the disk's own sectors. `appendable_gb`
/// multiplies that span by the sector size the table reported. Counting a
/// 4Kn disk in 512-byte sectors would report eight times the room it has.
/// The editor form would then accept a root the disk cannot hold, the
/// deletes would be written, and `sfdisk` would shrink the root silently.
/// The conservative arithmetic exists to make that outcome impossible.
#[test]
fn the_derived_span_counts_in_the_disks_own_sectors() {
    let root = scratch("disk-sectors");
    let image = root.join("disk.img");
    let bytes = 8u64 * 1024 * 1024 * 1024;
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(bytes))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    assert_eq!(super::disk_sectors(&disk, 512), bytes / 512);
    assert_eq!(super::disk_sectors(&disk, 4096), bytes / 4096);
    // A table that named no sector size is read as 512. The division never
    // takes a zero divisor.
    assert_eq!(super::disk_sectors(&disk, 0), bytes / 512);
    // The room comes out the same at either sector size, because the span
    // and the multiplier count in the same unit.
    let room = |sector: u64| {
        DiskTable {
            first: 2048,
            last: super::disk_sectors(&disk, sector).saturating_sub(34),
            sector,
            label: String::new(),
            slots: Vec::new(),
        }
        .appendable_gb(&[])
    };
    assert_eq!(room(512), room(4096));
    assert_eq!(room(512), 8);
    let _ = std::fs::remove_dir_all(&root);
}

/// A node the disk already made is taken at once, without spending the
/// settle budget. `settled_node` looks before it sleeps.
#[test]
fn an_existing_node_is_taken_at_once() {
    let root = scratch("settled-node");
    let disk = root.join("disk.img");
    std::fs::File::create(&disk).expect("the disk");
    let node = root.join("disk.img1");
    std::fs::File::create(&node).expect("the node");
    assert_eq!(
        super::settled_node(&disk.to_string_lossy(), 1),
        Some(node.to_string_lossy().into_owned())
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A partition the disk never names stops the install. A file grows no device
/// nodes, so `settled_node` spends its budget and refuses, and the install
/// stops before the recipe names a node that is not there.
#[test]
fn a_cut_whose_node_never_appears_stops_the_install() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("cut-node-wait");
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(8 * 1024 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let mut layout = CustomLayout {
        disk: disk.clone(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
        }],
        ..Default::default()
    };
    layout.confirmed = Some(super::disk_state(&disk).expect("the disk state"));
    let err = super::cut_partitions(&mut layout).expect_err("no node grows on a file");
    assert!(err.contains("no device node appeared"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A layout that plans no delete and no create runs no `sfdisk` at all. The
/// disk named here does not exist, so any `sfdisk` call or disk lock would
/// fail. A layout that only assigns and formats reaches fisherman with the
/// partition table it found.
#[test]
fn a_layout_that_cuts_nothing_writes_no_partition_table() {
    let mut layout = CustomLayout::empty("/dev/definitely-not-a-disk");
    assert!(super::cut_partitions(&mut layout).is_ok());
}
