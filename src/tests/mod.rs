pub(crate) use super::*;
use std::io::Write as _;

mod answers;
mod backend;
mod boot;
mod discover;
mod disks;
mod editor;
mod etcwrite;
mod fdisk;
mod form;
mod lock;
mod panel;
mod payload;
mod recipe;
mod run;
mod table;
mod volumes;

/// This dump was measured on 2026-09-19 with util-linux 2.41.5, and the keys
/// the dump reader skips are trimmed out. The disk is a 200 MiB GPT whose
/// slot 2 was deleted and re-taken by a 10M append. That append landed after
/// the last partition and never in the 20M hole the delete left. A dump
/// reader that drifts from `sfdisk` fails here and never on a disk.
const MEASURED_DUMP: &str = "\
label: gpt
device: /dev/vda
unit: sectors
first-lba: 2048
last-lba: 409566
sector-size: 512

/dev/vda1 : start=        2048, size=       40960, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=\"one\"
/dev/vda2 : start=      124928, size=       20480, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=\"small\"
/dev/vda3 : start=       83968, size=       40960, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=\"three\"
";

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("tect-install-{name}.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch tree");
    root
}

/// Holds the payload the form tests ask their questions against. The image,
/// hostname, filesystem and bootloader match `EMITTED`, which describes the
/// same Debian target.
fn a_payload() -> Payload {
    let payload = Payload {
        recipe: "/mnt/tect/install-recipe.json".into(),
        image: "ghcr.io/tectonic-os/deb2:latest".to_string(),
        hostname: "deb2".to_string(),
        filesystem: "ext4".to_string(),
        bootloader: "grub2".to_string(),
        boot: String::new(),
        composefs: false,
        reserve: std::sync::OnceLock::new(),
        luks_initramfs: true,
    };
    // `image_gb` runs podman, which no test does, so the fixture presets
    // the 10 GB `root_reserve` falls back to for an unmeasurable image.
    payload
        .reserve
        .set(10)
        .expect("a reserve nothing has read yet");
    payload
}

/// Holds what `emit::recipe::build` emits for a Debian target, which is what
/// the payload root carries at install time.
const EMITTED: &str = r#"{
  "image": "ghcr.io/tectonic-os/deb2:latest",
  "targetImgref": "ghcr.io/tectonic-os/deb2:latest",
  "composeFsBackend": true,
  "genericImage": true,
  "bootloader": "grub2",
  "filesystem": "ext4",
  "hostname": "deb2",
  "user": { "groups": ["sudo"] },
  "additionalImageStores": ["/var/lib/tectonic/store"]
}"#;
