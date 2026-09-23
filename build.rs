//! Links the installer against the media's own `libfdisk`.
//!
//! The release container ships `libfdisk.so.1` and no development package, so
//! `-l fdisk` finds no unversioned `libfdisk.so` to resolve. Naming the
//! versioned file outright links the same `DT_NEEDED` entry without one.
//! Measured 2026-09-22 in `centos-bootc:stream10`, where `libfdisk-devel` lives
//! in the disabled CRB repository. Full record
//! `measurements/libfdisk-linkage.md`.

use std::path::Path;

/// Holds the directories a distribution puts `libfdisk.so.1` in. CentOS and
/// Fedora both use `/usr/lib64` on the architectures the installer releases
/// for. Debian multiarch is second so a build there resolves too.
const SEARCHED: [&str; 3] = [
    "/usr/lib64",
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
];

/// Names the file the linker is pointed at.
///
/// libfdisk versions its exports. It carries 13 version nodes of its own,
/// `FDISK_2.26` through `FDISK_2_41`, and 281 of its 294 defined symbols bind
/// to one. Recounted 2026-09-22 on `libfdisk-2.41.5-1.fc44`: the 14 entries
/// `readelf -V` lists as definitions are those 13 plus the `libfdisk.so.1`
/// soname, which is not a node a symbol can bind to.
/// Symbol versioning is backward compatible, so the build must run against the
/// oldest libfdisk the installer supports, which is the 2.40.2 of the release
/// container. A binary built against a newer node refuses to start on an older
/// base with `version FDISK_2_41 not found`, which fails loudly rather than
/// silently. Measured 2026-09-22 with `readelf -V`.
const SONAME: &str = "libfdisk.so.1";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for directory in SEARCHED {
        let library = Path::new(directory).join(SONAME);
        if library.exists() {
            println!("cargo:rustc-link-arg={}", library.display());
            return;
        }
    }
    // A build host without the library fails here rather than at the link
    // step, because the linker's own diagnostic names a missing symbol and
    // not the missing package.
    panic!("{SONAME} is in none of {SEARCHED:?}, so the partition backend cannot link");
}
