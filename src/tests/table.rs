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

/// `regions` reports every free span the surviving partitions leave, in
/// sector order. `/dev/vda1` spans 2048..43007 and `/dev/vda3` spans
/// 83968..124927, so the measured table leaves a 20M hole between them and the
/// tail after `/dev/vda2`. A deleted partition merges the spans on both its
/// sides, because nothing separates them any more.
#[test]
fn the_free_regions_are_the_spans_the_surviving_partitions_leave() {
    let table = super::table_of(MEASURED_DUMP);
    let spans = |deletes: &[&str]| -> Vec<(u64, u64)> {
        table
            .regions(&deletes.iter().map(|at| at.to_string()).collect::<Vec<_>>())
            .into_iter()
            .map(|region| (region.start, region.sectors))
            .collect()
    };
    // Slot 2 sits between slots 1 and 3 by start, though the dump lists it
    // second, so the head gap runs 2048..43007.
    assert_eq!(spans(&[]), [(43008, 40960), (145408, 264159)]);
    // Deleting the highest-ending partition gives its room back to the tail.
    assert_eq!(spans(&["/dev/vda2"]), [(43008, 40960), (124928, 284639)]);
    // Deleting the partition below the hole merges the two into one span.
    assert_eq!(spans(&["/dev/vda1"]), [(2048, 81920), (145408, 264159)]);
    // Clearing the disk gives the whole usable span back.
    let all: Vec<String> = table.slots.iter().map(|slot| slot.node.clone()).collect();
    let all: Vec<&str> = all.iter().map(String::as_str).collect();
    assert_eq!(spans(&all), [(2048, 407519)]);
}

/// A create is placed in the lowest free region with room for its offset and
/// size, so a hole a delete left is used before the tail. `sfdisk --append`
/// with an explicit `start=` takes that exact sector, so the start this
/// returns is the partition's first sector.
#[test]
fn a_create_takes_the_lowest_region_with_room_for_it() {
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
                start: 10 * gb + 2048,
                sectors: 20 * gb,
            },
            Slot {
                node: "/dev/vda3".to_string(),
                number: 3,
                start: 30 * gb + 2048,
                sectors: 10 * gb,
            },
        ],
    };
    let create = |gb: u64, offset: u64| Created {
        gb,
        offset,
        target: "/".to_string(),
        fstype: "btrfs".to_string(),
        ..Default::default()
    };
    let deletes = vec!["/dev/vda2".to_string()];
    // The freed 20 GB hole sits below the tail, so a 5 GB create takes it and
    // a 50 GB create, which no hole holds, takes the tail.
    let (starts, _) = place_creates(&table, &deletes, &[create(5, 0), create(50, 0)])
        .expect("both creates fit the disk");
    let hole = table.aligned(&Region {
        start: 10 * gb + 2048,
        sectors: 20 * gb,
    });
    let tail = table.aligned(&Region {
        start: 40 * gb + 2048,
        sectors: 60 * gb - 2047,
    });
    assert_eq!(starts, [hole.start, tail.start]);
    // An offset moves the create into its region by that many whole GB.
    let (starts, _) = place_creates(&table, &deletes, &[create(5, 3)]).expect("a 5 GB create");
    assert_eq!(starts, [hole.start + 3 * gb]);
    // A create no region holds refuses with the largest region the plan could
    // not use, and nothing is placed.
    let room = create_rooms(&table, &[], &[]);
    assert_eq!(room.largest, 59);
    assert_eq!(place_creates(&table, &[], &[create(60, 0)]).err(), Some(59));
}

/// The create window opens on the room the plan leaves. The size field takes
/// the first free region's whole GB, which is the hole a delete made, and the
/// largest region is the most one create can take.
#[test]
fn the_create_window_opens_on_the_room_the_plan_leaves() {
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
                start: 10 * gb + 2048,
                sectors: 20 * gb,
            },
            Slot {
                node: "/dev/vda3".to_string(),
                number: 3,
                start: 30 * gb + 2048,
                sectors: 10 * gb,
            },
        ],
    };
    // No delete leaves the tail alone, and a delete runs its hole first.
    assert_eq!(create_rooms(&table, &[], &[]).first, 59);
    let deletes = vec!["/dev/vda2".to_string()];
    let rooms = create_rooms(&table, &deletes, &[]);
    assert_eq!(rooms.first, 19, "the freed hole is the default size");
    assert_eq!(rooms.largest, 59, "the tail is still the largest region");
    // A create already in the plan spends the hole before the next window
    // opens, so the default size is what the hole still has.
    let held = [Created {
        gb: 5,
        ..Default::default()
    }];
    assert_eq!(create_rooms(&table, &deletes, &held).first, 14);
    // A plan that no longer fits offers no room at all.
    let too_big = [Created {
        gb: 95,
        ..Default::default()
    }];
    let rooms = create_rooms(&table, &deletes, &too_big);
    assert_eq!(rooms.first, 0);
    assert!(rooms.largest < 95, "{}", rooms.largest);
}

/// The size opens on the first free region in sector order, which is where a
/// create lands, and not on the smallest region. A window that offered the
/// smallest would default a size taken from a region the create never uses.
#[test]
fn the_create_size_opens_on_the_first_region_not_the_smallest() {
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
                sectors: 30 * gb,
            },
            Slot {
                node: "/dev/vda2".to_string(),
                number: 2,
                start: 30 * gb + 2048,
                sectors: 8 * gb,
            },
            Slot {
                node: "/dev/vda3".to_string(),
                number: 3,
                start: 60 * gb,
                sectors: 30 * gb,
            },
        ],
    };
    // The gap between slots 2 and 3 comes first and holds 21 GB; the tail
    // holds 9 GB and comes last.
    let rooms = create_rooms(&table, &[], &[]);
    assert_eq!(rooms.first, 21, "the first region in sector order");
    assert_eq!(rooms.largest, 21);
    // A placement for the default size lands in the first region.
    let placed = place_creates(
        &table,
        &[],
        &[Created {
            gb: rooms.first,
            ..Default::default()
        }],
    );
    let (start, _) = placed.expect("the default size fits the first region");
    let first = table.aligned(&Region {
        start: 38 * gb + 2048,
        sectors: 22 * gb - 2048,
    });
    assert_eq!(start, [first.start]);
}

/// A create is placed in an aligned span, because `sfdisk` aligns a partition
/// to 1 MiB and shaves an unaligned end. `aligned` trims both ends to the
/// 1 MiB grid the disk's own sector size defines.
#[test]
fn a_region_is_trimmed_to_the_alignment_sfdisk_writes() {
    let table = super::table_of(MEASURED_DUMP);
    let region = super::Region {
        start: 43008,
        sectors: 40960,
    };
    // 43008 and 83968 are both 21 and 41 whole MiB, so the hole survives whole.
    assert_eq!(
        table.aligned(&region),
        super::Region {
            start: 43008,
            sectors: 40960
        }
    );
    // The span below starts one sector late and ends one sector early, so the
    // aligned span starts a MiB later and ends a MiB earlier.
    let off = super::Region {
        start: 43009,
        sectors: 40958,
    };
    assert_eq!(
        table.aligned(&off),
        super::Region {
            start: 45056,
            sectors: 36864
        }
    );
    // A span too short to hold one aligned sector offers nothing.
    assert_eq!(
        table.aligned(&super::Region {
            start: 43009,
            sectors: 100
        }),
        super::Region {
            start: 45056,
            sectors: 0
        }
    );
    assert_eq!(table.placeable_gb(&region), 40960 * 512 / 1_000_000_000);
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
    // `regions` filters on the same key and takes its own arm of `cleared_by`,
    // so it needs a partial delete of its own. Slot 2 sits at the tail, so
    // removing it merges its room into the span before it. A comparison that
    // matched nothing would leave the two spans apart and refuse a create the
    // disk has the room for.
    let highest = vec!["/dev/sda2".to_string()];
    assert_eq!(
        table.regions(&highest),
        [super::Region {
            start: 43008,
            sectors: 366559
        }]
    );
    assert_ne!(table.regions(&highest), table.regions(&[]));
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
    assert_eq!(
        table.regions(&all),
        [super::Region {
            start: 2048,
            sectors: 407519
        }]
    );
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
            ..Default::default()
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
    let script = "label: gpt\nsize=2G, name=one\nsize=2G, name=two\nsize=2G, name=three\n";
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
    let deleted = before
        .slots
        .iter()
        .find(|slot| slot.number == 2)
        .expect("the middle slot")
        .clone();
    let layout = CustomLayout {
        disk: disk.clone(),
        deletes: vec![format!("{disk}2")],
        creates: vec![
            Created {
                gb: 1,
                target: "/boot/efi".to_string(),
                fstype: "fat32".to_string(),
                device: String::new(),
                ..Default::default()
            },
            Created {
                gb: 1,
                target: "/".to_string(),
                fstype: "btrfs".to_string(),
                device: String::new(),
                ..Default::default()
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
    // whole new partition. The original slot 2 held 2G and this one holds
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
    // Both creates land in the hole the delete left, one after the other,
    // and neither reaches the surviving partition below. The cut names each
    // create's first sector, and the region's alignment bounds what it writes.
    let three = after
        .slots
        .iter()
        .find(|slot| slot.number == 3)
        .expect("slot 3");
    assert_eq!(
        two.start, deleted.start,
        "the first create took the deleted slot's own sector"
    );
    let four = after
        .slots
        .iter()
        .find(|slot| slot.number == 4)
        .expect("slot 4");
    assert!(four.start >= two.start + two.sectors, "the creates overlap");
    assert!(
        four.start + four.sectors <= three.start,
        "the creates ran past the hole into the surviving partition"
    );
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

/// A size no free region can hold stops the install before the first write.
/// `sfdisk` accepts an oversized request, shrinks the partition and exits 0,
/// so a root quietly smaller than the user asked for would otherwise reach
/// fisherman on a disk that had already been cut. The table is compared whole
/// afterwards, because the refusal has to come before the deletes.
#[test]
fn a_create_bigger_than_the_free_region_stops_the_install() {
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
    let before = super::disk_table(&disk).expect("the table");
    let layout = CustomLayout {
        disk: disk.clone(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let err = super::apply_cuts(&layout).expect_err("a 1 GB partition on a 200 MiB disk");
    assert!(err.contains("no room"), "{err}");
    assert_eq!(
        super::disk_table(&disk).expect("the table"),
        before,
        "the refusal left the disk untouched"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The names the plan carries reach the disk. A partition that exists is
/// renamed through `sfdisk --part-label`, and a planned partition carries its
/// name in the cut script. Both are read back with `sfdisk --dump`, because
/// `disk_table` reads no names.
#[test]
fn the_cut_writes_the_planned_names() {
    let Ok(sfdisk) = Command::new("sfdisk").arg("--version").output() else {
        return;
    };
    if !sfdisk.status.success() {
        return;
    }
    let root = scratch("cut-names");
    let image = root.join("disk.img");
    std::fs::File::create(&image)
        .and_then(|file| file.set_len(8 * 1024 * 1024 * 1024))
        .expect("a backing file");
    let disk = image.to_string_lossy().to_string();
    let script = "label: gpt\nsize=1G, name=one\nsize=1G, name=two\n";
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

    // A rename on a partition that exists and a name on a planned one.
    let layout = CustomLayout {
        disk: disk.clone(),
        creates: vec![Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            label: "root".to_string(),
            device: String::new(),
            ..Default::default()
        }],
        renames: vec![Rename {
            partition: format!("{disk}1"),
            label: "renamed".to_string(),
        }],
        ..Default::default()
    };
    let cut = super::apply_cuts(&layout).expect("the cut");
    assert_eq!(cut, [3]);
    let dump = Command::new("sfdisk")
        .args(["--dump", &disk])
        .output()
        .expect("a dump");
    let dump = String::from_utf8_lossy(&dump.stdout);
    for (number, label) in [(1, "renamed"), (3, "root")] {
        let line = dump
            .lines()
            .find(|line| line.starts_with(&format!("{disk}{number} ")))
            .unwrap_or_else(|| panic!("no line for slot {number} in:\n{dump}"));
        assert!(
            line.contains(&format!("name=\"{label}\"")),
            "slot {number}: {line}"
        );
    }
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
            ..Default::default()
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
            ..Default::default()
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
    let err = super::check_created_slots(&layout, &[2], &short)
        .expect_err("a slot sfdisk shrank must stop the install");
    assert!(err.contains("did not say so"), "{err}");
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
        filled.wrong_label_for(&[], 1, 0).as_deref(),
        Some(copy::custom_not_gpt("dos").as_str())
    );
    // A planned name is refused on the same label, because a `dos` partition
    // carries no name for `sfdisk --part-label` to write.
    assert_eq!(
        filled.wrong_label_for(&[], 0, 1).as_deref(),
        Some(copy::custom_not_gpt("dos").as_str())
    );
    // A plan with no create and no name appends nothing. The label rule
    // guards the append numbering and the names alone, so a `dos` disk whose
    // partitions the user only mounts and formats is not refused.
    assert!(filled.wrong_label_for(&[], 0, 0).is_none());
    // Clearing the disk is allowed. That path writes a fresh `label: gpt`,
    // so the disk comes out GPT and is never appended to as `dos`.
    let all = vec!["/dev/vda1".to_string()];
    assert!(filled.wrong_label_for(&all, 1, 1).is_none());
    // Cleared, the free span is the whole usable disk. It is not measured
    // from a partition the cut will remove.
    assert_eq!(
        filled.regions(&all),
        [super::Region {
            start: 2048,
            sectors: 407519
        }]
    );
    // A disk with no table carries an empty label, which is not refused.
    let blank = DiskTable {
        first: 2048,
        last: 409566,
        sector: 512,
        ..Default::default()
    };
    assert!(blank.wrong_label_for(&[], 1, 0).is_none());
    assert_eq!(
        blank.regions(&[]),
        [super::Region {
            start: 2048,
            sectors: 407519
        }]
    );
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
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    // The plan deletes the 90 GB partition and cuts a 2 GB ESP beside an
    // 87 GB root. The freed span is 90 GB less the 1 MiB alignment, and
    // `placeable_gb` floors that to 89.
    let freed = vec!["/dev/vda2".to_string()];
    let rooms = create_rooms(&table, &freed, &[]);
    assert_eq!(rooms.first, 89, "the create window's size default");
    assert_eq!(rooms.largest, 89);
    let ok = plan(
        vec!["/dev/vda2"],
        vec![(2, "/boot/efi", "fat32"), (87, "/", "btrfs")],
    );
    assert_eq!(layout_short_of(&ok, false, Some(&table), 0), None);
    // One GB over the room is refused. The ESP was placed first, so the room
    // the refusal states is what the freed span leaves after it.
    let over = plan(
        vec!["/dev/vda2"],
        vec![(2, "/boot/efi", "fat32"), (88, "/", "btrfs")],
    );
    assert_eq!(
        layout_short_of(&over, false, Some(&table), 0).as_deref(),
        Some(copy::custom_too_big(87).as_str())
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
            ..Default::default()
        },
        Created {
            gb: 1,
            target: "/".to_string(),
            fstype: "btrfs".to_string(),
            device: String::new(),
            ..Default::default()
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

/// The derived span counts the disk's own sectors. `placeable_gb` multiplies a
/// span by the sector size the table reported. Counting a 4Kn disk in
/// 512-byte sectors would report eight times the room it has. The editor form
/// would then accept a root the disk cannot hold, the deletes would be
/// written, and `sfdisk` would shrink the root silently. The conservative
/// arithmetic exists to make that outcome impossible.
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
        let table = DiskTable {
            first: 2048,
            last: super::disk_sectors(&disk, sector).saturating_sub(34),
            sector,
            label: String::new(),
            slots: Vec::new(),
        };
        let region = table.regions(&[]);
        table.placeable_gb(&region[0])
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
            ..Default::default()
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

/// Holds a 100 GB disk with three partitions of 10, 20 and 10 GB and the
/// tail free.
fn three_slots() -> DiskTable {
    let sector = 512u64;
    let gb = 1_000_000_000 / sector;
    let slot = |number: usize, start: u64, sectors: u64| Slot {
        node: format!("/dev/vda{number}"),
        number,
        start,
        sectors,
    };
    DiskTable {
        first: 2048,
        last: 100 * gb,
        sector,
        label: "gpt".to_string(),
        slots: vec![
            slot(1, 2048, 10 * gb),
            slot(2, 10 * gb + 2048, 20 * gb),
            slot(3, 30 * gb + 2048, 10 * gb),
        ],
    }
}

/// Names the slots as `slot_names` gives them once the plan deletes slot 2.
fn names() -> Vec<(usize, String)> {
    vec![(1, "efi".to_string()), (3, "vda3".to_string())]
}

#[test]
fn a_freed_hole_is_bounded_by_the_partitions_beside_it() {
    let deletes = vec!["/dev/vda2".to_string()];
    let holes = create_holes(&three_slots(), &deletes, &[], &names());
    let sides: Vec<(Option<&str>, Option<&str>)> = holes
        .iter()
        .map(|hole| (hole.before.as_deref(), hole.after.as_deref()))
        .collect();
    assert_eq!(sides, [(Some("efi"), Some("vda3")), (Some("vda3"), None)]);
}

/// A create already in the plan fills the front of the freed hole, so the
/// next window's region starts after it.
#[test]
fn a_planned_create_bounds_the_region_it_leaves() {
    let deletes = vec!["/dev/vda2".to_string()];
    let held = [Created {
        gb: 5,
        ..Default::default()
    }];
    let mut names = names();
    names.push((2, "new".to_string()));
    let holes = create_holes(&three_slots(), &deletes, &held, &names);
    assert_eq!(holes[0].before.as_deref(), Some("new"));
    assert_eq!(holes[0].after.as_deref(), Some("vda3"));
}
