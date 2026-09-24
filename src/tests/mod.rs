pub(crate) use super::*;
use std::io::Write as _;

mod answers;
mod backend;
mod boot;
mod discover;
mod disks;
mod editor;
mod esp;
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
        esp_entries: std::sync::OnceLock::new(),
    };
    // `image_gb` runs podman, which no test does, so the fixture presets
    // the 10 GB `root_reserve` falls back to for an unmeasurable image.
    payload
        .reserve
        .set(10)
        .expect("a reserve nothing has read yet");
    // The probe into the image is a podman run as well, so the fixture holds
    // the entries a payload writes and a case that needs them sets its own.
    payload
        .esp_entries
        .set(Vec::new())
        .expect("entries nothing has read yet");
    payload
}

/// Holds the payload `a_payload` describes with the ESP entries one image
/// writes, for the table cases that draw what replaces an entry.
fn a_payload_writing(entries: &[&str]) -> Payload {
    let mut payload = a_payload();
    payload.esp_entries = std::sync::OnceLock::new();
    payload
        .esp_entries
        .set(entries.iter().map(|entry| entry.to_string()).collect())
        .expect("entries nothing has read yet");
    payload
}

/// Names one partition the walk read, for the label cases that place a
/// system on a device.
fn part(device: &str, fstype: &str) -> Partition {
    Partition {
        device: device.to_string(),
        fstype: fstype.to_string(),
        ..Default::default()
    }
}

/// Names one system row a partition carries, for the cases that build a scan
/// by hand. The link stays empty, because these cases place no system.
fn label(name: &str) -> Label {
    Label {
        name: name.to_string(),
        link: String::new(),
    }
}

/// Builds the scan a table case draws from the disk rows and partitions it
/// holds. The table and the walk come back empty, because these cases draw
/// the picture alone.
fn scan_of(disks: &[(&str, &str)], parts: &[(&str, Vec<Partition>)]) -> Scan {
    Scan {
        unread: Vec::new(),
        disks: disks
            .iter()
            .map(|(device, detail)| DiskScan {
                device: device.to_string(),
                detail: detail.to_string(),
                partitions: parts
                    .iter()
                    .find(|(at, _)| at == device)
                    .map(|(_, parts)| parts.to_vec())
                    .unwrap_or_default(),
                carries: false,
                table: Ok(DiskTable::default()),
                labels: Vec::new(),
                keys: Discovered::default(),
            })
            .collect(),
    }
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
