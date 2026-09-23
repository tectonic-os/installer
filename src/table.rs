use super::*;
use std::io::Write as _;

/// One entry of a disk's partition table. The editor screens read `lsblk`,
/// which reports sizes and filesystems. The append arithmetic needs the sector
/// a partition starts at, and `discover::partitions` never asks `lsblk` for
/// it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Slot {
    pub(crate) node: String,
    pub(crate) number: usize,
    pub(crate) start: u64,
    pub(crate) sectors: u64,
}

/// A disk's partition table, read through `libfdisk`. The span and each slot
/// size count sectors, because that is the unit libfdisk reports and the
/// append arithmetic multiplies by the sector size. A disk over 2 TiB holds
/// more sectors than a `u32` counts, which is why the reader is not
/// `sfdisk --json`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DiskTable {
    /// `first` and `last` bound the sectors a partition may occupy, and
    /// `last` is inclusive. `sector` is the sector size in bytes, which the
    /// byte arithmetic multiplies by.
    pub(crate) first: u64,
    pub(crate) last: u64,
    pub(crate) sector: u64,
    /// The table label the reader reports, `gpt` or `dos`, and empty for a
    /// disk that carries no table. `wrong_label_for` refuses a create on a
    /// `dos` label the plan does not clear.
    pub(crate) label: String,
    pub(crate) slots: Vec<Slot>,
}

/// The disk the user confirmed, with the table it carried then. `rdev`
/// identifies a block device. `dev` and `ino` keep the same identity check
/// working for the file-backed tables the tests cut instead.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DiskState {
    pub(crate) rdev: u64,
    pub(crate) dev: u64,
    pub(crate) ino: u64,
    pub(crate) table: DiskTable,
}

impl DiskTable {
    /// The sectors an appended partition can be cut from. The region runs
    /// from the end of the highest-ending partition that survives the planned
    /// deletes to `last`.
    ///
    /// The region is smaller than the disk's free space, and smaller than
    /// `sfdisk` sometimes uses. Measured 2026-09-19 on util-linux 2.41.5
    /// (`measurements/sfdisk-append.md`), `--append` takes the highest free
    /// region that fits. A 10M append skipped a free 20M hole at sector 43008
    /// and took the tail at 124928. A 50M append with only a 15M tail did land
    /// in a 244M hole. Measuring the tail alone is therefore conservative. It
    /// can refuse a create that would have fitted in a hole. It can never
    /// accept one that overruns. A refused layout costs the user a retry. An
    /// accepted impossible one costs the disk, because `sfdisk` shrinks the
    /// partition silently.
    pub(crate) fn appendable(&self, deletes: &[String]) -> u64 {
        // A plan that removes every partition writes a fresh GPT label
        // instead of appending, so the room is the whole usable disk. A blank
        // disk takes this branch too, because it leaves no surviving partition
        // to measure from.
        let gone = deleted_slots(deletes);
        let end = match self.cleared_by(deletes) {
            true => self.first,
            false => self
                .slots
                .iter()
                .filter(|slot| !gone.contains(&slot.number))
                .map(|slot| slot.start.saturating_add(slot.sectors))
                .max()
                .unwrap_or(self.first)
                .max(self.first),
        };
        self.last.saturating_add(1).saturating_sub(end)
    }

    /// Whether the planned deletes remove every partition in this table. A
    /// cleared table lets the cut write a fresh GPT label instead of appending
    /// into the surviving partitions.
    fn cleared_by(&self, deletes: &[String]) -> bool {
        let gone = deleted_slots(deletes);
        self.slots.is_empty() || self.slots.iter().all(|slot| gone.contains(&slot.number))
    }

    /// Why this disk cannot take a created partition beside the ones it keeps.
    /// Only a GPT disk is appended to. A `dos` label numbers an appended
    /// partition by rules `appended_slots` does not predict, and this
    /// project's images boot from an ESP that a GPT type GUID identifies.
    /// Clearing the disk is always allowed, because that writes a new GPT
    /// label instead of appending to the old one.
    pub(crate) fn wrong_label_for(&self, deletes: &[String], creates: usize) -> Option<String> {
        // A layout that creates nothing never appends, so the append
        // numbering this guard protects does not apply to it. Its deletes
        // still run `sfdisk`, so the reason is the numbering and not an
        // untouched partition table.
        if creates == 0 {
            return None;
        }
        match self.label.is_empty() || self.label == "gpt" || self.cleared_by(deletes) {
            true => None,
            false => Some(copy::custom_not_gpt(&self.label)),
        }
    }

    /// The same room in whole GB, which is the unit the editor screen asks in
    /// and the refusal states. The installer draws every size in decimal GB.
    pub(crate) fn appendable_gb(&self, deletes: &[String]) -> u64 {
        self.appendable(deletes)
            .saturating_mul(self.sector)
            .saturating_div(1_000_000_000)
    }

    /// The slot numbers that survive the planned deletes, which is what
    /// `appended_slots` predicts the new numbers from.
    pub(crate) fn surviving(&self, deletes: &[String]) -> Vec<usize> {
        let gone = deleted_slots(deletes);
        self.slots
            .iter()
            .filter(|slot| !gone.contains(&slot.number))
            .map(|slot| slot.number)
            .collect()
    }
}

/// The slot numbers a delete list names.
///
/// The two sides of a delete are named by different tools. `layout.deletes`
/// holds the `NAME` `lsblk --paths` gave a partition. `DiskTable.slots[].node`
/// holds the `fdisk_partname` name, which the reader takes from libfdisk and
/// `sfdisk --dump` prints for its own rows. The two strings differ whenever
/// the user names the disk by a `/dev/disk/by-id` or `/dev/disk/by-path`
/// path. lsblk answers `/dev/nvme0n1p3` there and `fdisk_partname` answers
/// `<disk>-part3`. Measured 2026-09-22 against lsblk 2.41.5 and libfdisk
/// 2.41.5.
///
/// A `/dev/mapper` disk differs for its own reason. lsblk reports a
/// device-mapper device under its `/dev/mapper` name and keeps the kernel
/// name in `KNAME`, which `discover::partitions` does not ask for, and kpartx
/// names the maps it creates with a delimiter the user chooses. So the two
/// strings differ there without lsblk ever answering a kernel name.
///
/// What holds in every case is narrower. The trailing digits of `NAME` are
/// the partition number, so the slot number is the one identity both tools
/// agree on, and it is what `drawn_slots` and `appended_slots` already use.
///
/// Comparing the strings made every delete on such a disk match nothing. The
/// plan then appended after partitions it was about to remove, and
/// `check_created_slots` refused the install once `sfdisk` had cut the disk.
///
/// The trailing digits give the slot number on the branches that separate the
/// number from the name. The digit branch inserts `p`, so partition 1 of
/// `/dev/disk/by-uuid/abcd1234` is `/dev/disk/by-uuid/abcd1234p1`, and the
/// `-part` branch ends in a letter. The `<disk><N>` probe is the exception,
/// because it answers a node that can belong to another map. With
/// `/dev/mapper/pool21` present, `/dev/mapper/pool2` partition 1 reads as slot
/// 21, while `kpartx` and `lsblk` name the real partition `pool2p1` as slot 1.
/// A delete on such a disk then matches nothing, and the plan can append after
/// a partition it removes. Filed in `BACKLOG.md`.
///
/// If a delete carries no trailing digits, or digits that overflow `usize`,
/// this drops it. `apply_cuts` refuses that same entry before it writes the
/// disk, and both sides ask `partition_number`, so neither can accept an
/// entry the other rejects.
fn deleted_slots(deletes: &[String]) -> Vec<usize> {
    deletes
        .iter()
        .filter_map(|device| partition_number(device).ok())
        .collect()
}

/// The slot numbers `sfdisk --append` hands `count` created partitions.
/// `sfdisk` takes the lowest free number each time, measured 2026-09-19 on
/// util-linux 2.41.5. With slots 1 and 3 present and slot 2 deleted, the
/// appended partition became slot 2. See `measurements/sfdisk-append.md`.
///
/// The rule takes the surviving slot numbers rather than a `DiskTable`,
/// because two callers hold two sources for them and must not answer
/// differently. The editor screen holds `lsblk`'s partitions, and the cut
/// holds the slot numbers `disk_table` read. A node drawn on the screen that
/// the cut does not produce is the defect this rule exists to avoid.
///
/// The rule holds for GPT only. A `dos` label appending beside an extended
/// partition numbered the new partition 5 rather than the predicted 4, so a
/// non-GPT disk is cleared to a fresh GPT label and never appended to.
pub(crate) fn appended_slots(taken: &[usize], count: usize) -> Vec<usize> {
    let mut given: Vec<usize> = Vec::new();
    let mut number = 1;
    while given.len() < count {
        if !taken.contains(&number) {
            given.push(number);
        }
        number += 1;
    }
    given
}

/// Reads the key lines and partition lines of `sfdisk --dump`. An unrecognised
/// line is skipped instead of refused, because `--dump` also carries
/// `label-id`, `device` and `grain`, and a new key in a later util-linux must
/// not stop the comparison.
///
/// The dump parser serves `sfdisk_table` and the tests alone, because the
/// install reads its table through `disk_table`.
#[cfg(test)]
pub(crate) fn table_of(dump: &str) -> DiskTable {
    let mut table = DiskTable::default();
    for line in dump.lines() {
        let line = line.trim();
        let number = |value: &str| value.trim().parse::<u64>().ok();
        if let Some(value) = line.strip_prefix("label:") {
            table.label = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("first-lba:") {
            table.first = number(value).unwrap_or_default();
        } else if let Some(value) = line.strip_prefix("last-lba:") {
            table.last = number(value).unwrap_or_default();
        } else if let Some(value) = line.strip_prefix("sector-size:") {
            table.sector = number(value).unwrap_or_default();
        } else if let Some((node, fields)) = line.split_once(" : ") {
            let node = node.trim().to_string();
            let Ok(at) = partition_number(&node) else {
                continue;
            };
            let field = |key: &str| {
                fields
                    .split(',')
                    .filter_map(|field| field.trim().strip_prefix(key))
                    .find_map(|value| value.trim().parse::<u64>().ok())
            };
            table.slots.push(Slot {
                node,
                number: at,
                start: field("start=").unwrap_or_default(),
                sectors: field("size=").unwrap_or_default(),
            });
        }
    }
    // A disk with no table reports no sector size, and the append arithmetic
    // would then size every create at zero GB and refuse all of them. 512 is
    // what `sfdisk` itself assumes.
    if table.sector == 0 {
        table.sector = 512;
    }
    table
}

/// The path a device name is read at. A path outside `/dev` is returned
/// unchanged. In a debug build `env::dev` renames the `/dev` directory, so a
/// fixture run can hold a disk as a regular file. A released installer reads
/// the name itself, which keeps a fixture from aiming a real install at
/// another directory.
fn device_path(disk: &str) -> String {
    match (cfg!(debug_assertions), disk.strip_prefix("/dev/")) {
        (true, Some(name)) => env::dev().join(name).to_string_lossy().into_owned(),
        _ => disk.to_string(),
    }
}

/// The disk's table, read fresh. Every caller reads it again instead of
/// caching, because the editor screen is live and a disk the user swapped
/// carries different slots.
pub(crate) fn disk_table(disk: &str) -> Result<DiskTable, String> {
    fdisk::read_table(&device_path(disk))
}

/// The `sfdisk`-backed reader the install path ran before `libfdisk`. It
/// serves `src/tests/fdisk.rs` as the comparison side for `disk_table`, the
/// way `table_of` serves it as the dump parser. It goes when the two readers
/// stop being worth comparing.
#[cfg(test)]
pub(crate) fn sfdisk_table(disk: &str) -> Result<DiskTable, String> {
    let out = Command::new("sfdisk")
        .args(["--dump", disk])
        // `unpartitioned_dump` matches an English diagnostic, so the locale
        // is pinned here. An I/O or permission failure must never read as an
        // unpartitioned disk.
        .env("LC_ALL", "C")
        .output()
        .map_err(|err| format!("sfdisk --dump {disk}: {err}"))?;
    // An unpartitioned disk is not a failure here. It has no table to dump,
    // and the comparison cases cut one.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut table = match out.status.success() {
        true => table_of(&String::from_utf8_lossy(&out.stdout)),
        false if unpartitioned_dump(&stderr) => DiskTable {
            sector: 512,
            ..Default::default()
        },
        false => {
            return Err(format!("sfdisk --dump {disk}: {}", stderr.trim()));
        }
    };
    // `first-lba` and `last-lba` are GPT keys. A `dos` dump carries neither,
    // and a disk with no table carries neither. The fill below is what the
    // install reader did, and the comparison cases cover the blank and `dos`
    // disks that need it. The span is GPT's own, from the first aligned
    // sector to the last sector before the secondary header's 33-sector
    // reserve.
    if table.last == 0 {
        table.first = 2048;
        // `appendable_gb` multiplies this span by `table.sector`, so the span
        // counts the disk's own sectors. Counting a 4Kn disk in 512-byte
        // sectors reports eight times the room it has, which is the divergence
        // the 4Kn comparison case pins.
        table.last = disk_sectors(disk, table.sector).saturating_sub(34);
    }
    Ok(table)
}

/// `sfdisk --dump` exits non-zero for an unpartitioned disk, which is not a
/// failure this reader refuses.
#[cfg(test)]
fn unpartitioned_dump(stderr: &str) -> bool {
    stderr.contains("does not contain a recognized partition table")
}

pub(crate) fn disk_state(disk: &str) -> Result<DiskState, String> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::metadata(device_path(disk)).map_err(|err| format!("{disk}: {err}"))?;
    Ok(DiskState {
        rdev: metadata.rdev(),
        dev: metadata.dev(),
        ino: metadata.ino(),
        table: disk_table(disk)?,
    })
}

/// Holds an advisory BSD lock on the disk across the confirmation check and
/// every table write. Tools that honour `sfdisk --lock`, including udev, see
/// the same lock while this process owns the cut.
struct DiskLock(std::fs::File);

impl DiskLock {
    pub(crate) fn take(disk: &str) -> Result<Self, String> {
        use std::os::fd::AsRawFd as _;

        let file = std::fs::File::open(disk).map_err(|err| format!("{disk}: {err}"))?;
        let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if locked == 0 {
            Ok(Self(file))
        } else {
            Err(format!(
                "{disk} is busy changing its partition table; review it again"
            ))
        }
    }
}

impl Drop for DiskLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd as _;

        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// The disk's size in its own sectors, which gives the span a fresh GPT label
/// would have. `sfdisk_table` fills a missing span with this, and the tests
/// ask it directly. `lsblk` is how this crate asks the machine about disks,
/// so this asks it for the size in bytes.
#[cfg(test)]
pub(crate) fn disk_sectors(disk: &str, sector: u64) -> u64 {
    // `table_of` reads a missing sector size as 512, so this divides by 512
    // too rather than by zero.
    let sector = match sector {
        0 => 512,
        sector => sector,
    };
    let out = Command::new("lsblk")
        .args([
            "--bytes",
            "--nodeps",
            "--noheadings",
            "--output",
            "SIZE",
            disk,
        ])
        .output();
    let Ok(out) = out else { return 0 };
    if !out.status.success() {
        // `lsblk` does not know a file-backed image, so the image's own
        // length gives the sector count. The tests cut such an image.
        return std::fs::metadata(disk)
            .map(|at| at.len() / sector)
            .unwrap_or_default();
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .map(|bytes| bytes / sector)
        .unwrap_or_default()
}

/// The nodes the created partitions are predicted to carry once the cut has
/// run, in the order the user added them to the plan. The editor screen draws
/// these. The nodes are derived on every call and never stored, because
/// deleting another partition frees a slot and moves them. An answer keyed by
/// a node that moves lands on the wrong partition.
///
/// A prediction cannot know a node that exists only after the cut, which is
/// the case on a `/dev/mapper` disk, so the cut re-reads the node instead of
/// trusting this answer.
pub(crate) fn created_devices(disk: &str, taken: &[usize], count: usize) -> Vec<String> {
    appended_slots(taken, count)
        .into_iter()
        .map(|number| fdisk::partname(disk, number))
        .collect()
}

/// The surviving slot numbers as the editor screen holds them, which are
/// `lsblk`'s partitions for the chosen disk less the ones the plan removes.
pub(crate) fn drawn_slots(parts: &[Partition], deletes: &[String]) -> Vec<usize> {
    parts
        .iter()
        .filter(|part| !deletes.contains(&part.device))
        .filter_map(|part| partition_number(&part.device).ok())
        .collect()
}

/// The GPT type a created partition is cut with. Firmware finds the ESP by its
/// type GUID, so a partition the layout mounts at `/boot/efi` is cut as an
/// EFI System partition. A Linux filesystem type there would let the install
/// finish and leave the machine unable to boot. `U` and `L` are `sfdisk`'s own
/// shorthands, checked 2026-09-19 to expand to
/// `C12A7328-F81F-11D2-BA4B-00A0C93EC93B` and
/// `0FC63DAF-8483-4772-8E79-3D69D8477DE4`.
pub(crate) fn created_type(target: &str) -> &'static str {
    match target {
        "/boot/efi" => "U",
        _ => "L",
    }
}

/// Cuts the partitions the layout planned. The deletes run first, so their
/// slots are free for the appends to take. Every create then follows in one
/// `sfdisk` call, which hands them the lowest free numbers in order.
///
/// `run` calls this before it opens, formats or mounts any volume, because
/// every later step names devices that do not exist until this returns. A
/// layout that planned no delete and no create runs no `sfdisk`.
pub(crate) fn cut_partitions(layout: &mut CustomLayout) -> Result<(), String> {
    if layout.deletes.is_empty() && layout.creates.is_empty() {
        return Ok(());
    }
    let _lock = DiskLock::take(&layout.disk)?;
    let now = disk_state(&layout.disk)?;
    if layout.confirmed.as_ref() != Some(&now) {
        return Err(copy::table_changed(&layout.disk));
    }
    let numbers = apply_cuts(layout)?;
    // The node is read from the disk rather than taken from the pre-cut
    // prediction. A `/dev/mapper` disk names its kpartx node only after the
    // cut, and `fdisk::partname` answers the `-part<N>` fallback until a node
    // exists. The wait covers the gap between the `sfdisk` command returning
    // and `udev` making the node, which is milliseconds on a real machine.
    // The budget is generous, because a wrong answer refuses an install on a
    // disk that is already cut.
    for (create, number) in layout.creates.iter_mut().zip(numbers) {
        let Some(device) = settled_node(&layout.disk, number) else {
            return Err(format!(
                "partition {number} on {} was cut but no device node appeared \
                 for it; review the disk and install again",
                layout.disk
            ));
        };
        create.device = device;
    }
    Ok(())
}

/// The `sfdisk` half of the cut, without the wait for device nodes. It
/// returns the slot numbers `sfdisk` gave the created partitions, in plan
/// order, and refuses a create the disk did not hold. A test runs this
/// against a file-backed table, which grows no nodes, so the table it wrote
/// is the whole of what it did.
pub(crate) fn apply_cuts(layout: &CustomLayout) -> Result<Vec<usize>, String> {
    if layout.deletes.is_empty() && layout.creates.is_empty() {
        return Ok(Vec::new());
    }
    // The table is read before any write, because the editor screen predicted
    // the new slot numbers from the surviving slots. Reading the table after
    // the deletes would read that prediction off the change it predicts.
    let table = disk_table(&layout.disk)?;
    if let Some(why) = table.wrong_label_for(&layout.deletes, layout.creates.len()) {
        return Err(why);
    }
    // A table no partition survives is replaced instead of edited. The
    // replacement is one write rather than one delete per partition, and it
    // leaves no old label behind. It also makes a `dos` or unlabelled disk
    // usable, because the disk comes out GPT, which an ESP's type GUID and
    // this project's boot chain both need. A blank disk takes this branch too,
    // and the fresh label gives `--append` a table to append into.
    let cleared = table.cleared_by(&layout.deletes);
    // The cleared branch numbers from 1, however the old table numbered.
    let numbers = match cleared {
        true => appended_slots(&[], layout.creates.len()),
        false => appended_slots(&table.surviving(&layout.deletes), layout.creates.len()),
    };
    // Every delete is numbered before the disk is written, and before the
    // branch, so the guard does not depend on which arm runs. A delete the
    // installer cannot number used to fail inside the loop below, once the
    // deletes before it had already run, which left the disk part way through
    // a plan and reported only the entry it stopped on. `deleted_slots` drops
    // such an entry, so the editor screen drew a plan the loop would refuse.
    // `partition_number` is the one test of what is numberable, which is what
    // keeps the readers and this loop from disagreeing about it.
    let numbered = layout
        .deletes
        .iter()
        .map(|device| partition_number(device).map(|number| (device, number)))
        .collect::<Result<Vec<_>, _>>()?;
    match cleared {
        true => sfdisk_script(&layout.disk, &[], "label: gpt\n")?,
        false => {
            for (device, number) in numbered {
                let out = Command::new("sfdisk")
                    .args(["-q", "--delete", &layout.disk, &number.to_string()])
                    .output()
                    .map_err(|err| format!("sfdisk --delete: {err}"))?;
                if !out.status.success() {
                    return Err(format!(
                        "the cut could not remove {device}: {}\n\n{}",
                        String::from_utf8_lossy(&out.stderr).trim(),
                        copy::table_already_changed(&layout.disk)
                    ));
                }
            }
        }
    }
    if layout.creates.is_empty() {
        return Ok(Vec::new());
    }
    let script: String = layout
        .creates
        .iter()
        .map(|create| {
            format!(
                "size={}GB, type={}\n",
                create.gb,
                created_type(&create.target)
            )
        })
        .collect();
    sfdisk_script(&layout.disk, &["--append"], &script)?;
    // `sfdisk` does not refuse a size the disk cannot hold. Measured
    // 2026-09-19, `size=1GB` on a 200 MiB disk produced a 197M partition and
    // exited 0 with an empty stderr. The editor refuses the room before the
    // form closes, so a short partition here means that arithmetic was wrong.
    // A root quietly smaller than the user asked for, on a disk already cut,
    // is worth stopping the install for.
    let after = disk_table(&layout.disk)?;
    check_created_slots(layout, &numbers, &after)?;
    Ok(numbers)
}

/// `sfdisk` exits zero when an append names no partition at all. This reads
/// the table the cut just wrote and decides whether each planned slot exists.
///
/// The slot number is the key and not the node name. A created partition has
/// no node before the cut, so the screen can only predict the name, and a
/// `/dev/mapper` disk predicts a name the disk never gives it.
pub(crate) fn check_created_slots(
    layout: &CustomLayout,
    numbers: &[usize],
    after: &DiskTable,
) -> Result<(), String> {
    for (create, number) in layout.creates.iter().zip(numbers) {
        // Measured 2026-09-19, a `dos` label with four primaries takes a
        // fifth `--append`, exits 0 and writes no partition. The recipe would
        // then name a partition that is not there, so a missing slot fails
        // here. `cut_partitions` catches the missing node too, and the
        // file-backed path this function also serves has no node to wait for.
        let Some(slot) = after.slots.iter().find(|slot| slot.number == *number) else {
            return Err(format!(
                "partition {number} is not in the partition table the cut \
                 wrote; the disk numbered the new partitions differently than \
                 the screen drew them\n\n{}",
                copy::table_already_changed(&layout.disk)
            ));
        };
        let got = slot.sectors.saturating_mul(after.sector);
        let asked = create.gb.saturating_mul(1_000_000_000);
        // One percent of slack covers the 1 MiB alignment `sfdisk` rounds to.
        if got < asked.saturating_sub(asked / 100) {
            return Err(format!(
                "partition {number} was cut at {} GB, though the plan asked \
                 for {} GB; sfdisk shrank it to fit and did not say so\n\n{}",
                got / 1_000_000_000,
                create.gb,
                copy::table_already_changed(&layout.disk)
            ));
        }
    }
    Ok(())
}

/// Feeds `sfdisk` a script on stdin and fails with the diagnostic it printed.
/// The fresh label and the appends share this one function, because both must
/// report the same way. Both must also warn that the table may already have
/// changed, which is the difference between a refused install and a disk the
/// user has to recover.
fn sfdisk_script(disk: &str, args: &[&str], script: &str) -> Result<(), String> {
    let mut command = Command::new("sfdisk");
    command.arg("-q");
    command.args(args);
    command.arg(disk);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("sfdisk {disk}: {err}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| format!("sfdisk {disk}: no stdin"))?
        .write_all(script.as_bytes())
        .map_err(|err| format!("sfdisk {disk}: {err}"))?;
    let out = child
        .wait_with_output()
        .map_err(|err| format!("sfdisk {disk}: {err}"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "the cut could not write the partition table on {disk}: {}\n\nthe \
             partition table on {disk} may already have been changed",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// How long a freshly cut partition has to appear, and how often
/// `settled_node` looks for it. Five seconds is far past what a machine takes
/// and far short of what the user would call hung.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(100);
const SETTLE_TRIES: usize = 50;

/// The node partition `number` has once the cut has reached the kernel, or
/// `None` when no node appears within the settle budget.
///
/// `fdisk::partname` is asked again on every look instead of once, because
/// its answer depends on which nodes exist. A partition the cut is about to
/// make has no node, so the first answer is the `-part<N>` fallback. A
/// `/dev/mapper` disk makes the node after the cut, under `<disk>p<N>` or
/// `<disk><N>`, and a later look finds it.
pub(crate) fn settled_node(disk: &str, number: usize) -> Option<String> {
    for _ in 0..SETTLE_TRIES {
        let node = fdisk::partname(disk, number);
        if Path::new(&node).exists() {
            return Some(node);
        }
        std::thread::sleep(SETTLE);
    }
    None
}
