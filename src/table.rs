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
    /// disk that carries no table. `wrong_label_for` refuses a create or a
    /// planned name on a `dos` label the plan does not clear.
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

/// One free span of a disk's partition table, in the disk's own sectors.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Region {
    pub(crate) start: u64,
    pub(crate) sectors: u64,
}

impl DiskTable {
    /// The free spans this table leaves once the planned deletes have run, in
    /// sector order. A delete merges the free space on both its sides, because
    /// the partition that separated them is gone.
    ///
    /// The spans are raw. A create is placed in one through `aligned`, which
    /// rounds a span to the sectors `sfdisk` can actually fill.
    pub(crate) fn regions(&self, deletes: &[String]) -> Vec<Region> {
        let gone = deleted_slots(deletes);
        let mut survivors: Vec<&Slot> = match self.cleared_by(deletes) {
            true => Vec::new(),
            false => self
                .slots
                .iter()
                .filter(|slot| !gone.contains(&slot.number))
                .collect(),
        };
        survivors.sort_by_key(|slot| slot.start);
        let mut regions = Vec::new();
        let mut at = self.first;
        for slot in survivors {
            if slot.start > at {
                regions.push(Region {
                    start: at,
                    sectors: slot.start - at,
                });
            }
            at = at.max(slot.start.saturating_add(slot.sectors));
        }
        if at <= self.last {
            regions.push(Region {
                start: at,
                sectors: self.last.saturating_add(1) - at,
            });
        }
        regions
    }

    /// Rounds one raw span to the sectors a create may fill. `sfdisk` aligns a
    /// partition to 1 MiB and shaves an unaligned end, so a span the placement
    /// accepts must be one a whole-GB partition fits inside. Trimming both
    /// ends here keeps the arithmetic from accepting a span the cut would
    /// silently shrink into.
    pub(crate) fn aligned(&self, region: &Region) -> Region {
        let align = self.alignment();
        let start = region.start.div_ceil(align) * align;
        let end = (region.start.saturating_add(region.sectors)) / align * align;
        Region {
            start,
            sectors: end.saturating_sub(start),
        }
    }

    /// The sectors `sfdisk` aligns a partition to, which is 1 MiB. A table
    /// that reports no sector size is read in 512-byte sectors.
    fn alignment(&self) -> u64 {
        match self.sector {
            0 => 2048,
            sector => (1_048_576 / sector).max(1),
        }
    }

    /// Gives the whole GB any offset-and-size pair the create window accepts
    /// can be placed in, which is the unit that window asks in. Two sectors
    /// cover the alignment `place_creates` adds to the offset and the size, so
    /// a pair this room accepts never overruns the region it lands in. The
    /// division floors, so a span smaller than a GB offers no room.
    pub(crate) fn placeable_gb(&self, region: &Region) -> u64 {
        let region = self.aligned(region);
        let sector = self.sector.max(1);
        region
            .sectors
            .saturating_mul(sector)
            .saturating_sub(2 * sector.saturating_sub(1))
            / 1_000_000_000
    }

    /// Whether the planned deletes remove every partition in this table. A
    /// cleared table lets the cut write a fresh GPT label instead of appending
    /// into the surviving partitions.
    fn cleared_by(&self, deletes: &[String]) -> bool {
        let gone = deleted_slots(deletes);
        self.slots.is_empty() || self.slots.iter().all(|slot| gone.contains(&slot.number))
    }

    /// Why this disk cannot take a created partition or a planned name beside
    /// the partitions it keeps. Only a GPT disk is appended to and only a GPT
    /// partition carries a label. A `dos` label numbers an appended partition
    /// by rules `appended_slots` does not predict, and this project's images
    /// boot from an ESP that a GPT type GUID identifies. Clearing the disk is
    /// always allowed, because that writes a new GPT label instead of
    /// appending to the old one.
    pub(crate) fn wrong_label_for(
        &self,
        deletes: &[String],
        creates: usize,
        renames: usize,
    ) -> Option<String> {
        // A layout that cuts nothing and names nothing never uses the GPT
        // features this guard protects, so the guard does not apply to it.
        // Its deletes still run `sfdisk`, so the reason is the append
        // numbering and not an untouched partition table.
        if creates == 0 && renames == 0 {
            return None;
        }
        match self.label.is_empty() || self.label == "gpt" || self.cleared_by(deletes) {
            true => None,
            false => Some(copy::custom_not_gpt(&self.label)),
        }
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

/// Holds the room a plan's next create has: the first free region's whole GB,
/// which is the size the create window opens on, and the largest free
/// region's whole GB, which is the most one create can take.
pub(crate) struct Rooms {
    pub(crate) first: u64,
    pub(crate) largest: u64,
}

/// Places every create in a free region, in the order the plan holds them, and
/// returns each create's starting sector and the regions left over. A create
/// takes the lowest region with room for its offset and size, so the hole a
/// delete left is used before the tail.
///
/// `Err` carries the largest free region in whole GB, which is what a refusal
/// states: the plan could not use it.
pub(crate) fn place_creates(
    table: &DiskTable,
    deletes: &[String],
    creates: &[Created],
) -> Result<(Vec<u64>, Vec<Region>), u64> {
    let sector = table.sector.max(1);
    let mut free = table.regions(deletes);
    let mut starts = Vec::new();
    for create in creates {
        let before = create.offset.saturating_mul(1_000_000_000).div_ceil(sector);
        let size = create.gb.saturating_mul(1_000_000_000).div_ceil(sector);
        let chosen = free.iter().position(|region| {
            let aligned = table.aligned(region);
            before.saturating_add(size) <= aligned.sectors
        });
        let Some(at) = chosen else {
            let largest = free
                .iter()
                .map(|region| table.placeable_gb(region))
                .max()
                .unwrap_or(0);
            return Err(largest);
        };
        let region = free.remove(at);
        let aligned = table.aligned(&region);
        let start = aligned.start.saturating_add(before);
        starts.push(start);
        // The head and the tail the create did not take stay free, so a later
        // create can use them.
        if aligned.start < start {
            free.push(Region {
                start: aligned.start,
                sectors: start - aligned.start,
            });
        }
        let end = start.saturating_add(size);
        let last = region.start.saturating_add(region.sectors);
        if end < last {
            free.push(Region {
                start: end,
                sectors: last - end,
            });
        }
        free.sort_by_key(|region| region.start);
    }
    Ok((starts, free))
}

/// Gives the room the create window offers, with the plan's own creates placed
/// first, so a second create sees what the first left. The size opens on the
/// first free region with room for a whole GB, which is the hole a delete made
/// or the tail, and the largest region is the most one create can take.
pub(crate) fn create_rooms(table: &DiskTable, deletes: &[String], creates: &[Created]) -> Rooms {
    let free = match place_creates(table, deletes, creates) {
        Ok((_, free)) => free,
        // A plan that no longer fits has no room to offer. The window's own
        // refusal states the region that could not be used.
        Err(largest) => return Rooms { first: 0, largest },
    };
    // `place_creates` leaves the regions in start order, and the size opens on
    // the first one. A region smaller than a GB offers no whole-GB create, so
    // the size opens on the first region a create could actually take.
    let rooms = free
        .iter()
        .map(|region| table.placeable_gb(region))
        .filter(|room| *room > 0);
    let mut first = 0;
    let mut largest = 0;
    for room in rooms {
        if first == 0 {
            first = room;
        }
        largest = largest.max(room);
    }
    Rooms { first, largest }
}

/// Gives each free region the plan leaves, with the partitions on either side
/// of it, for the bar the create window draws. The plan's own creates are
/// placed first, as `create_rooms` places them. `names` holds the name the
/// layout table draws each slot number by.
pub(crate) fn create_holes(
    table: &DiskTable,
    deletes: &[String],
    creates: &[Created],
    names: &[(usize, String)],
) -> Vec<common::ui::Hole> {
    let Ok((starts, free)) = place_creates(table, deletes, creates) else {
        return Vec::new();
    };
    let sector = table.sector.max(1);
    let gone = deleted_slots(deletes);
    let mut taken: Vec<(usize, u64, u64)> = table
        .slots
        .iter()
        .filter(|slot| !gone.contains(&slot.number))
        .map(|slot| {
            (
                slot.number,
                slot.start,
                slot.start.saturating_add(slot.sectors),
            )
        })
        .collect();
    let numbers = appended_slots(&table.surviving(deletes), creates.len());
    for ((number, start), create) in numbers.into_iter().zip(starts).zip(creates) {
        let size = create.gb.saturating_mul(1_000_000_000).div_ceil(sector);
        taken.push((number, start, start.saturating_add(size)));
    }
    // A slot the names miss still bounds its region. It draws by its number,
    // because a missing box would read as the start or the end of the disk.
    let named = |slot: &(usize, u64, u64)| {
        names
            .iter()
            .find(|(number, _)| *number == slot.0)
            .map_or_else(|| slot.0.to_string(), |(_, name)| name.clone())
    };
    free.iter()
        .filter_map(|region| {
            let gb = table.placeable_gb(region);
            let end = region.start.saturating_add(region.sectors);
            let before = taken
                .iter()
                .filter(|slot| slot.2 <= region.start)
                .max_by_key(|slot| slot.2);
            let after = taken
                .iter()
                .filter(|slot| slot.1 >= end)
                .min_by_key(|slot| slot.1);
            (gb > 0).then(|| common::ui::Hole {
                before: before.map(named),
                after: after.map(named),
                gb,
            })
        })
        .collect()
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
        // `placeable_gb` multiplies this span by `table.sector`, so the span
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
/// `sfdisk` call, which hands them the lowest free numbers in order, and every
/// rename follows that.
///
/// `run` calls this before it opens, formats or mounts any volume, because
/// every later step names devices that do not exist until this returns. A
/// layout that planned no delete, no create and no rename runs no `sfdisk`.
pub(crate) fn cut_partitions(layout: &mut CustomLayout) -> Result<(), String> {
    if layout.deletes.is_empty() && layout.creates.is_empty() && layout.renames.is_empty() {
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
    if layout.deletes.is_empty() && layout.creates.is_empty() && layout.renames.is_empty() {
        return Ok(Vec::new());
    }
    // The table is read before any write, because the editor screen predicted
    // the new slot numbers and the placement from it. Reading the table after
    // the deletes would read that prediction off the change it predicts.
    let table = disk_table(&layout.disk)?;
    if let Some(why) =
        table.wrong_label_for(&layout.deletes, layout.creates.len(), layout.renames.len())
    {
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
    // Every delete and every rename is numbered before the disk is written, so
    // an entry the installer cannot number refuses while the table is whole.
    // `deleted_slots` drops such an entry, so the editor screen drew a plan
    // the loop would refuse. `partition_number` is the one test of what is
    // numberable, which is what keeps the readers and this loop from
    // disagreeing about it.
    let numbered = layout
        .deletes
        .iter()
        .map(|device| partition_number(device).map(|number| (device, number)))
        .collect::<Result<Vec<_>, _>>()?;
    let renamed = layout
        .renames
        .iter()
        .map(|rename| partition_number(&rename.partition).map(|number| (rename, number)))
        .collect::<Result<Vec<_>, _>>()?;
    // The placement is computed before the first write, so a plan the disk
    // cannot hold refuses while the table is whole. `layout_short_of` refused
    // the same plan on the form's own table; a different answer here means the
    // disk moved while the plan was reviewed.
    let (starts, _) = place_creates(&table, &layout.deletes, &layout.creates)
        .map_err(|room| copy::custom_too_big(room))?;
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
    if !layout.creates.is_empty() {
        let script: String = layout
            .creates
            .iter()
            .zip(&starts)
            .map(|(create, start)| {
                let name = match create.label.is_empty() {
                    true => String::new(),
                    false => format!(", name=\"{}\"", create.label),
                };
                format!(
                    "start={start}, size={}GB, type={}{name}\n",
                    create.gb,
                    created_type(&create.target)
                )
            })
            .collect();
        sfdisk_script(&layout.disk, &["--append"], &script)?;
        // `sfdisk` does not refuse a size the disk cannot hold, and it shrinks
        // the partition silently. The editor refuses the room before the form
        // closes, so a short partition here means that arithmetic was wrong.
        // A root quietly smaller than the user asked for, on a disk already
        // cut, is worth stopping the install for.
        let after = disk_table(&layout.disk)?;
        check_created_slots(layout, &numbers, &after)?;
    }
    // A rename runs last, so a name that cannot be written is reported once
    // every planned partition exists. It changes no slot, so the create checks
    // above read the same table either way.
    for (rename, number) in renamed {
        part_label(&layout.disk, number, &rename.label)?;
    }
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

/// Writes one existing partition's label through `sfdisk --part-label`. The
/// label is a GPT name, which no boot chain reads, so this runs after the
/// deletes and the creates and reports a failure with the warning that the
/// table may already have moved. The slot number names the partition, because
/// a `/dev/mapper` disk names its partition nodes by rules the screen cannot
/// predict.
fn part_label(disk: &str, number: usize, label: &str) -> Result<(), String> {
    let out = Command::new("sfdisk")
        .args(["-q", "--part-label", disk, &number.to_string(), label])
        .output()
        .map_err(|err| {
            format!(
                "sfdisk --part-label: {err}\n\n{}",
                copy::table_already_changed(disk)
            )
        })?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "the cut could not name partition {number} on {disk}: {}\n\n{}",
            String::from_utf8_lossy(&out.stderr).trim(),
            copy::table_already_changed(disk)
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
