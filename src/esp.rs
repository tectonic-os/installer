//! The systems an EFI System Partition carries, what each one boots, and the
//! entries an install replaces or removes.

use super::*;

/// Reports whether a filesystem is one this project's ESP is made of.
pub(crate) fn is_fat(fstype: &str) -> bool {
    ["vfat", "fat", "fat32"].contains(&fstype)
}

/// Names how one loader entry finds the filesystem it boots. A loader searches
/// by filesystem UUID or by filesystem label, and a BLS entry names the
/// container its root lives in beside the root itself.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Link {
    /// Names a filesystem the loader searches by UUID.
    Uuid(String),
    /// Names a filesystem the loader searches by label.
    Label(String),
}

/// Holds one system row a mounted filesystem carries, with every link its
/// loader names in the order the search runs. The candidates are read before
/// any partition is compared, because one may name a container that is closed
/// while another names the filesystem inside it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Carried {
    pub(crate) name: String,
    pub(crate) links: Vec<Link>,
}

/// Holds one system row under the partition that carries it, with the
/// partition it boots where the walk could place it. An empty link marks a
/// system the walk cannot place, and no install replaces or removes one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Label {
    pub(crate) name: String,
    pub(crate) link: String,
}

/// Holds the entries one ESP loses before the install writes its own, because
/// the systems they boot are rewritten or deleted.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EspRemoval {
    pub(crate) partition: String,
    pub(crate) entries: Vec<String>,
}

/// Names the vendor directory Windows writes its boot manager under. The
/// install itself is named `WINDOWS` by the GPT types its disk carries.
const MICROSOFT: &str = "Microsoft";

/// Reads the vendor directories one ESP carries, each with the filesystem its
/// loader boots. `EFI/BOOT` holds the fallback loader every ESP carries, so
/// it names no system on its own.
pub(crate) fn entries(at: &Path) -> Vec<Carried> {
    let Some(efi) = inside(at, Path::new("EFI")).filter(|path| path.is_dir()) else {
        return Vec::new();
    };
    let Ok(listed) = std::fs::read_dir(&efi) else {
        return Vec::new();
    };
    // The entries this ESP's own loader reads name what systemd-boot boots
    // when they live on the ESP rather than on a separate /boot.
    let entries = loader_links(at);
    let mut found: Vec<Carried> = listed
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| !name.eq_ignore_ascii_case("BOOT"))
        .map(|name| {
            let links = vendor_links(&efi, &name);
            // A Microsoft directory boots Windows, which no file on the ESP
            // names. The GPT types name the Windows install, so the loader
            // entries are never its fallback.
            let fallback = links.is_empty() && !name.eq_ignore_ascii_case(MICROSOFT);
            Carried {
                name,
                links: match fallback {
                    true => entries.clone(),
                    false => links,
                },
            }
        })
        .collect();
    found.sort_by(|one, other| one.name.cmp(&other.name));
    found
}

/// Reads what one vendor directory's loader boots. bootupd writes the boot
/// filesystem's UUID into `bootuuid.cfg`, and the static `grub.cfg` searches
/// for it by that UUID or by the boot label.
fn vendor_links(efi: &Path, name: &str) -> Vec<Link> {
    let dir = efi.join(name);
    let mut links = Vec::new();
    if let Some(uuid) = read(&dir, "bootuuid.cfg").and_then(|text| bootuuid(&text)) {
        links.push(Link::Uuid(uuid));
    }
    if let Some(link) = read(&dir, "grub.cfg").and_then(|text| searched(&text)) {
        links.push(link);
    }
    links
}

/// Reads one regular file under a directory the walk resolved. A FIFO, a
/// device and a symlink out of the mount are not files to read.
fn read(dir: &Path, name: &str) -> Option<String> {
    let file = inside_file(dir, Path::new(name))?;
    std::fs::read_to_string(&file).ok()
}

/// Reads the boot filesystem UUID bootupd writes into an ESP's
/// `bootuuid.cfg`. The file holds one `set BOOT_UUID="<uuid>"` line.
pub(crate) fn bootuuid(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let said = line.trim().strip_prefix("set BOOT_UUID=")?;
        named(said)
    })
}

/// Reads the filesystem a GRUB `search` line looks for. The line names it by
/// UUID or by label, and a value the build left as a shell variable names
/// nothing this walk can resolve.
pub(crate) fn searched(text: &str) -> Option<Link> {
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let command = words.next().unwrap_or("");
        let rest: Vec<&str> = words.collect();
        if command == "search.fs_uuid" {
            if let Some(uuid) = rest.first().and_then(|value| named(value)) {
                return Some(Link::Uuid(uuid));
            }
        }
        if command != "search" {
            continue;
        }
        for (at, word) in rest.iter().enumerate() {
            if !matches!(*word, "--fs-uuid" | "--label") {
                continue;
            }
            let value = name_after(rest.get(at + 1..).unwrap_or_default());
            match (*word, value) {
                ("--fs-uuid", Some(uuid)) => return Some(Link::Uuid(uuid)),
                ("--label", Some(label)) => return Some(Link::Label(label)),
                _ => {}
            }
        }
    }
    None
}

/// Reads the name a `search` command looks for, which is its first token that
/// is not an option. `--set` takes an optional variable name, and that name
/// is never the one being searched for.
fn name_after(words: &[&str]) -> Option<String> {
    let mut at = 0;
    while at < words.len() {
        let word = words[at];
        at += 1;
        if word == "--set" {
            if words.get(at).is_some_and(|value| !value.starts_with("--")) {
                at += 1;
            }
            continue;
        }
        if word.starts_with("--") {
            continue;
        }
        return named(word);
    }
    None
}

/// Reads one loader value that names a filesystem. A value held in a shell
/// variable names nothing yet, and neither does an empty one.
fn named(said: &str) -> Option<String> {
    let value = said.trim().trim_matches(['"', '\'']);
    (!value.is_empty() && !value.contains('$')).then(|| value.to_string())
}

/// Reads what the loader entries on one filesystem boot, in the order the
/// search runs: the root's UUID first, then the container the root lives in.
/// systemd-boot reads `loader/entries` on its own ESP, and the BLS entry names
/// the root by `root=UUID=` and the container by `rd.luks.uuid=`.
fn loader_links(at: &Path) -> Vec<Link> {
    for file in loader_files(at) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let links = bls_links(&text);
        if !links.is_empty() {
            return links;
        }
    }
    Vec::new()
}

/// Gives the loader entries one mounted filesystem carries, the ESP's own
/// directory first and a separate `/boot`'s second, each sorted by name.
pub(crate) fn loader_files(at: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in ["loader/entries", "boot/loader/entries"] {
        let Some(entries) = inside(at, Path::new(dir)).filter(|path| path.is_dir()) else {
            continue;
        };
        let Ok(listed) = std::fs::read_dir(&entries) else {
            continue;
        };
        let mut files: Vec<PathBuf> = listed
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|kind| kind == "conf"))
            .filter_map(|entry| inside_file(at, &Path::new(dir).join(entry.file_name())))
            .collect();
        files.sort();
        found.extend(files);
    }
    found
}

/// Reads what one BLS entry opens, in the order the search runs. A signed UKI
/// carries its own cmdline and names neither, so it links to nothing.
pub(crate) fn bls_links(text: &str) -> Vec<Link> {
    let words: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("options "))
        .flat_map(str::split_whitespace)
        .collect();
    let mut links: Vec<Link> = words
        .iter()
        .filter_map(|word| word.strip_prefix("root=UUID="))
        .map(|uuid| Link::Uuid(uuid.to_string()))
        .collect();
    links.extend(
        words
            .iter()
            .filter_map(|word| word.strip_prefix("rd.luks.uuid="))
            .map(|uuid| Link::Uuid(uuid.strip_prefix("luks-").unwrap_or(uuid).to_string())),
    );
    links
}

/// Reads the entry directory names an image's staged EFI payloads carry. The
/// install writes them into the ESP, so they are what replaces the entries it
/// removes. `EFI/BOOT` is the fallback directory every ESP carries and is not
/// a system of its own.
pub(crate) fn entry_names(listed: &str) -> Vec<String> {
    let mut names: Vec<String> = listed
        .lines()
        .filter_map(|line| line.trim().rsplit('/').next())
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("BOOT"))
        .map(str::to_string)
        .collect();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup_by(|one, other| one.eq_ignore_ascii_case(other));
    names
}

/// Resolves each walked label's link to a partition the scan read. The first
/// candidate that names one wins, so a BLS entry whose root filesystem sits in
/// a closed container falls back to the container it names. A Microsoft
/// directory takes the `windows` link when its own files name none.
pub(crate) fn linked(
    walked: Vec<(String, Vec<Carried>)>,
    partitions: &[Partition],
    windows: Option<&str>,
) -> Vec<(String, Vec<Label>)> {
    walked
        .into_iter()
        .map(|(at, carried)| {
            let rows = carried
                .into_iter()
                .map(|carried| {
                    let link = carried.links.iter().find_map(|link| match link {
                        Link::Uuid(uuid) => partitions.iter().find(|part| {
                            !part.uuid.is_empty() && part.uuid.eq_ignore_ascii_case(uuid)
                        }),
                        Link::Label(label) => partitions.iter().find(|part| {
                            !part.label.is_empty() && part.label.eq_ignore_ascii_case(label)
                        }),
                    });
                    let link = match link {
                        Some(part) => part.device.clone(),
                        None if carried.name.eq_ignore_ascii_case(MICROSOFT) => {
                            windows.unwrap_or_default().to_string()
                        }
                        None => String::new(),
                    };
                    Label {
                        name: carried.name,
                        link,
                    }
                })
                .collect();
            (at, rows)
        })
        .collect()
}

/// Names the ESP entries the plan replaces, so the install removes them
/// before it writes its own. Only the partition the plan assigns `/boot/efi`
/// receives new entries, so only its old entries are removed. A formatted ESP
/// has already lost them.
pub(crate) fn removals(scan: &Scan, layout: &CustomLayout) -> Vec<EspRemoval> {
    let mut found = Vec::new();
    for disk in &scan.disks {
        for (at, labels) in &disk.labels {
            let Some(partition) = disk.partitions.iter().find(|part| &part.device == at) else {
                continue;
            };
            let receives = layout
                .answer(partition)
                .is_some_and(|mounted| mounted.target == "/boot/efi");
            if !is_fat(&partition.fstype) || !receives || layout.formatted(at) {
                continue;
            }
            let entries: Vec<String> = labels
                .iter()
                .filter(|label| layout.entry_gone(&label.link))
                .map(|label| label.name.clone())
                .collect();
            if !entries.is_empty() {
                found.push(EspRemoval {
                    partition: at.clone(),
                    entries,
                });
            }
        }
    }
    found
}

/// Removes the ESP entries the plan replaces. The plan's ESP is kept, so this
/// mounts it read-write, removes one vendor directory per entry and unmounts.
/// A failure stops the install, which is why the confirmation named every
/// entry it would remove.
pub(crate) fn remove(layout: &CustomLayout) -> Result<(), String> {
    for removal in &layout.esp {
        let mut mounts = Mounts::default();
        let at = mount_rw(&removal.partition, &mut mounts)?;
        let efi = at.join("EFI");
        for entry in &removal.entries {
            remove_vendor(&efi, entry)
                .map_err(|why| format!("{why}\n\n{}", copy::table_already_changed(&layout.disk)))?;
        }
        // The unmount reports its failure, because the install stops here and
        // the next step is fisherman over a disk this process still holds.
        mounts.release(&at)?;
    }
    Ok(())
}

/// Removes one vendor directory under a mounted ESP. A symlink is unlinked
/// rather than followed, so the removal reaches nothing outside the ESP. The
/// name comes from a directory listing, and a name that is not one path
/// element is refused rather than joined.
pub(crate) fn remove_vendor(efi: &Path, entry: &str) -> Result<(), String> {
    let name = Path::new(entry);
    let one = name.components().count() == 1
        && matches!(
            name.components().next(),
            Some(std::path::Component::Normal(_))
        );
    if !one {
        return Err(copy::esp_entry_name(entry));
    }
    let said = efi.join(name);
    let meta = std::fs::symlink_metadata(&said)
        .map_err(|err| copy::esp_entry_missing(&said.display().to_string(), &err.to_string()))?;
    let removed = match meta.is_dir() {
        true => std::fs::remove_dir_all(&said),
        false => std::fs::remove_file(&said),
    };
    removed.map_err(|err| copy::esp_entry_remove(&said.display().to_string(), &err.to_string()))
}
