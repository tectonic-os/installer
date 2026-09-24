//! Reads a disk's partition table through `libfdisk` instead of `sfdisk`.
//!
//! The installer ran `sfdisk --dump` and parsed its text. libfdisk answers the
//! same questions through the library util-linux builds `sfdisk` itself on, so
//! the numbers come from one implementation rather than from a text format that
//! a later util-linux may extend. `build.rs` states how the library is linked.
//!
//! Every entry point here takes a device path and returns an owned
//! [`DiskTable`], because the editor screen re-reads a live disk and never
//! caches one.
//!
//! [`read_table`] is the install's only table reader. `table.rs` reaches it
//! through `disk_table`, and the `sfdisk`-backed reader it replaced lives
//! there for the comparison tests alone. [`partname`] names the nodes the
//! screen predicts and the nodes the cut re-reads.
//!
//! On a disk with no label the two readers take their sector size from
//! different places. The `sfdisk` reader in `table.rs` falls back to 512
//! because `sfdisk --dump` prints no `sector-size` line, and this module asks
//! libfdisk for the device's own size. A regular file cannot carry a 4Kn
//! sector size, so the case needs a `losetup -b 4096` device.
//!
//! `a_4kn_disk_with_no_label_reports_the_same_room_through_both` pins the
//! residual difference between the two readers on the unlabelled disk, and
//! this module offers the smaller room there, which is the safe side. The
//! test needs root, so it is `#[ignore]`d with its reason.
//!
//! The same disk carrying a `dos` label is unmeasured, and the readers can
//! differ there as well. The `sfdisk` reader's span fallback counts 1 MiB as
//! 2048 sectors however large the disk's sectors are, and this module counts
//! the alignment in the disk's own sectors. The gap is filed in `BACKLOG.md`.

use super::*;
use std::ffi::{c_char, c_int, c_ulong, c_void, CStr, CString};

// `fdisk_sector_t` is `uint64_t` and `fdisk_get_sector_size` returns
// `unsigned long`, which differ on 32-bit targets. The installer releases for
// `x86_64` and `aarch64` only, and both are LP64.
unsafe extern "C" {
    fn fdisk_new_context() -> *mut c_void;
    fn fdisk_unref_context(cxt: *mut c_void);
    fn fdisk_disable_dialogs(cxt: *mut c_void, disable: c_int) -> c_int;
    fn fdisk_assign_device(cxt: *mut c_void, fname: *const c_char, readonly: c_int) -> c_int;
    fn fdisk_deassign_device(cxt: *mut c_void, nosync: c_int) -> c_int;
    fn fdisk_has_label(cxt: *mut c_void) -> c_int;
    fn fdisk_get_label(cxt: *mut c_void, name: *const c_char) -> *mut c_void;
    fn fdisk_label_get_name(label: *mut c_void) -> *const c_char;
    fn fdisk_get_first_lba(cxt: *mut c_void) -> u64;
    fn fdisk_get_last_lba(cxt: *mut c_void) -> u64;
    fn fdisk_get_nsectors(cxt: *mut c_void) -> u64;
    fn fdisk_get_sector_size(cxt: *mut c_void) -> c_ulong;
    fn fdisk_get_partitions(cxt: *mut c_void, table: *mut *mut c_void) -> c_int;
    fn fdisk_unref_table(table: *mut c_void);
    fn fdisk_table_get_nents(table: *mut c_void) -> usize;
    fn fdisk_table_get_partition(table: *mut c_void, n: usize) -> *mut c_void;
    fn fdisk_partition_has_start(part: *mut c_void) -> c_int;
    fn fdisk_partition_has_size(part: *mut c_void) -> c_int;
    fn fdisk_partition_has_partno(part: *mut c_void) -> c_int;
    fn fdisk_partition_get_start(part: *mut c_void) -> u64;
    fn fdisk_partition_get_size(part: *mut c_void) -> u64;
    fn fdisk_partition_get_partno(part: *mut c_void) -> usize;
    fn fdisk_partname(dev: *const c_char, partno: usize) -> *mut c_char;
}

/// Holds a libfdisk context with a device assigned to it. `Drop` releases the
/// device and the context, so every way out of a read closes both. A partial
/// constructor leaves neither behind, because the context is assigned only
/// after it exists.
struct Context {
    cxt: *mut c_void,
    assigned: bool,
}

impl Context {
    /// Opens `disk` read-only. The dialogs are disabled because a later block
    /// may install an ask callback for the writer, and a callback that reaches
    /// a question during a read would block an install with nothing on screen.
    /// This module installs none, so every libfdisk question already returns
    /// `-EINVAL` and the call changes nothing today.
    fn read_only(disk: &str) -> Result<Self, String> {
        let path = CString::new(disk).map_err(|_| format!("{disk}: the path holds a NUL byte"))?;
        let cxt = unsafe { fdisk_new_context() };
        if cxt.is_null() {
            return Err(format!("libfdisk: no context for {disk}"));
        }
        let mut context = Self {
            cxt,
            assigned: false,
        };
        unsafe { fdisk_disable_dialogs(context.cxt, 1) };
        // libfdisk returns a negative errno. An unpartitioned disk assigns
        // without error and reports no label, so a failure here names a device
        // the installer cannot read at all.
        let rc = unsafe { fdisk_assign_device(context.cxt, path.as_ptr(), 1) };
        if rc < 0 {
            return Err(format!("libfdisk: cannot read {disk} ({rc})"));
        }
        context.assigned = true;
        Ok(context)
    }

    /// Names the label libfdisk recognised, `gpt` or `dos`. An unpartitioned
    /// disk carries no label and answers with an empty string, which is what
    /// `wrong_label_for` reads when it refuses a create or a planned name.
    fn label(&self) -> String {
        if unsafe { fdisk_has_label(self.cxt) } == 0 {
            return String::new();
        }
        let label = unsafe { fdisk_get_label(self.cxt, std::ptr::null()) };
        if label.is_null() {
            return String::new();
        }
        let name = unsafe { fdisk_label_get_name(label) };
        if name.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    }

    /// Lists the partitions the label declares, in the order libfdisk reports
    /// them. The order is the table's own and not a sector sort, because the
    /// editor screen shows slots as the disk holds them.
    fn slots(&self, disk: &str) -> Vec<Slot> {
        let mut table: *mut c_void = std::ptr::null_mut();
        if unsafe { fdisk_get_partitions(self.cxt, &mut table) } < 0 || table.is_null() {
            return Vec::new();
        }
        let mut slots = Vec::new();
        for index in 0..unsafe { fdisk_table_get_nents(table) } {
            let part = unsafe { fdisk_table_get_partition(table, index) };
            if part.is_null() {
                continue;
            }
            // `fdisk_get_partitions` adds only entries `fdisk_partition_is_used`
            // accepts, so free space never arrives here. A `dos` extended
            // container does arrive and does become a slot, because it reports
            // a start and a size like any other entry. That matches
            // `sfdisk --dump`, which lists the container too.
            //
            // The guard stands for an entry whose start, size or number
            // libfdisk left unset. Upstream requires `fdisk_partition_has_partno`
            // before the number is read, and the unset value is `(size_t)-1`,
            // which `number + 1` would wrap to 0 and name `/dev/sda0`.
            if unsafe { fdisk_partition_has_start(part) } == 0
                || unsafe { fdisk_partition_has_size(part) } == 0
                || unsafe { fdisk_partition_has_partno(part) } == 0
            {
                continue;
            }
            let number = unsafe { fdisk_partition_get_partno(part) };
            slots.push(Slot {
                // `fdisk_partition_get_partno` counts from zero and every
                // device node counts from one.
                node: partname(disk, number + 1),
                number: number + 1,
                start: unsafe { fdisk_partition_get_start(part) },
                sectors: unsafe { fdisk_partition_get_size(part) },
            });
        }
        unsafe { fdisk_unref_table(table) };
        slots
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        if self.assigned {
            // The context is read-only, so `fdisk_deassign_device` only
            // closes the descriptor and never reads the `nosync` argument.
            // The argument is passed as 1 to state the intent for the day a
            // writer opens a context read-write, where the other branch calls
            // `sync(2)` across the whole system.
            unsafe { fdisk_deassign_device(self.cxt, 1) };
        }
        unsafe { fdisk_unref_context(self.cxt) };
    }
}

/// Builds the device node for one partition of `disk`.
///
/// util-linux owns the rule and it has more than one branch. A `/dev` name
/// ending in a digit takes a `p`, so `/dev/nvme0n1` gives `/dev/nvme0n1p1`
/// while `/dev/vda` gives `/dev/vda1`. For a `/dev/disk/by-id`,
/// `/dev/disk/by-path` or `/dev/mapper` path, it answers `<disk><N>` or
/// `<disk>p<N>` when a node of that name exists, and the `-part` form when
/// neither does, so `/dev/mapper/mydisk` with no map behind it gives
/// `/dev/mapper/mydisk-part1`. **Those three prefixes and no others**: the
/// rest of `/dev/disk/by-*` takes the digit branch, so
/// `/dev/disk/by-uuid/abcd` gives `/dev/disk/by-uuid/abcd1`. A `/dev/dm-N`
/// path is resolved against the running device-mapper table first. Each node
/// probe reads the filesystem, so the answer depends on which nodes exist at
/// the moment of the call.
///
/// Measured 2026-09-22 against libfdisk 2.41.5 and `fdisk_partname` in
/// `libfdisk/src/utils.c`.
///
/// `created_devices` predicts the screen's nodes through this function, and
/// `settled_node` in `table.rs` asks it again after the cut. On a
/// `/dev/mapper` disk `kpartx` creates the node only after the cut, as
/// `<disk>p<N>` when the map name ends in a digit and `<disk><N>` otherwise,
/// so a created partition is predicted `-part<N>` and the later look reads
/// the node the cut really made.
pub(crate) fn partname(disk: &str, number: usize) -> String {
    let Ok(path) = CString::new(disk) else {
        return String::new();
    };
    let name = unsafe { fdisk_partname(path.as_ptr(), number) };
    if name.is_null() {
        return String::new();
    }
    let node = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    // `fdisk_partname` returns storage from `malloc`, and libfdisk offers no
    // free of its own for it.
    unsafe { libc::free(name.cast()) };
    node
}

/// Reads `disk`'s table. The result matches what `sfdisk_table` in `table.rs`
/// reports on a 512-byte disk and on any disk that carries a label. An
/// unlabelled 4Kn disk is the one measured case the two differ on, and the
/// module doc records the difference and the unmeasured `dos` case.
///
/// An unpartitioned disk is not a failure. It carries no label and the install
/// is most likely to cut partitions on it.
pub(crate) fn read_table(disk: &str) -> Result<DiskTable, String> {
    // The byte boundary util-linux aligns a first partition to. It is a length
    // rather than a sector count, because a sector count means a different
    // distance on a 4Kn disk than on a 512-byte one.
    const ALIGN: u64 = 1024 * 1024;
    let context = Context::read_only(disk)?;
    let sector = unsafe { fdisk_get_sector_size(context.cxt) } as u64;
    let mut table = DiskTable {
        // `fdisk_discover_topology` sets the sector size on every successful
        // assign, falling back to 512 for a regular file or a failed ioctl, so
        // a zero does not arrive here. The guard stands because the append
        // arithmetic multiplies by this number and a zero would size every
        // create at zero GB.
        sector: match sector {
            0 => 512,
            size => size,
        },
        label: context.label(),
        first: unsafe { fdisk_get_first_lba(context.cxt) },
        last: unsafe { fdisk_get_last_lba(context.cxt) },
        slots: context.slots(disk),
    };
    // The first and last usable LBA belong to a GPT header. libfdisk answers
    // for any label, and on a `dos` label or a disk with no label it answers
    // with the device's own ends. Those ends overstate the room by GPT's
    // 33-sector secondary header and the sector it sits on. A create sized
    // against them is accepted and then shrunk silently by the cut, which is
    // the one outcome the conservative arithmetic exists to prevent. Measured
    // 2026-09-22 against libfdisk 2.41.5 on a 200 MiB unlabelled image, which
    // reported 409599 where the GPT span ends at 409566.
    //
    // The first usable LBA is an alignment, so it counts the disk's own
    // sectors. util-linux aligns a partition to 1 MiB, which is 2048 sectors
    // on a 512-byte disk and 256 on a 4Kn one. Writing 2048 here reserved
    // 8 MiB at the front of a 4Kn disk and offered 7 MiB less room than the
    // `sfdisk` reader, which counts the same disk in 512-byte sectors and so
    // reaches 1 MiB from the same constant. Measured 2026-09-22 on a
    // `losetup -b 4096` device by `a_4kn_disk_with_no_label_reports_the_same_room_through_both`,
    // which is the case that found it.
    if table.label != "gpt" {
        table.first = ALIGN / table.sector;
        table.last = unsafe { fdisk_get_nsectors(context.cxt) }.saturating_sub(34);
    }
    Ok(table)
}
