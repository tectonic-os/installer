use super::*;

pub struct Encryption {
    pub kind: String,
    pub passphrase: String,
    /// Only `tpm2-luks-pin` needs a user PIN beside the TPM policy.
    /// `tpm2-luks` instead uses its passphrase as a generated recovery key.
    /// Separate fields prevent recovery material from being treated as a user
    /// PIN.
    pub pin: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CustomMount {
    pub(crate) partition: String,
    pub(crate) target: String,
    pub(crate) fstype: String,
    /// No reader takes this passphrase because `layout_short_of` refuses a
    /// manual `luks` format before installation.
    pub(crate) passphrase: String,
}

/// Existing partitions need `sfdisk --part-label`. Planned partitions carry
/// their labels in the cut script.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rename {
    pub(crate) partition: String,
    pub(crate) label: String,
}

/// Passphrases stay in memory, while key files remain named on the live system.
/// Neither credential is drawn, logged or written into a recipe.
#[derive(Clone, PartialEq)]
pub(crate) enum Key {
    Passphrase(String),
    File(PathBuf),
    /// A key read from the existing system loses its path when the discovery
    /// mount closes, so `cryptsetup` receives the retained bytes.
    Data(Vec<u8>),
}

impl Key {
    /// A named key file bypasses stdin, while an in-memory key must travel to
    /// `cryptsetup` through this process.
    /// The split prevents a file credential from being copied into memory.
    pub(crate) fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Passphrase(passphrase) => Some(passphrase.as_bytes()),
            Self::Data(bytes) => Some(bytes),
            Self::File(_) => None,
        }
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, form: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Passphrase(_) => form.write_str("Passphrase(***)"),
            Self::Data(_) => form.write_str("Data(***)"),
            Self::File(path) => form.debug_tuple("File").field(path).finish(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LuksOpen {
    pub(crate) partition: String,
    pub(crate) target: String,
    pub(crate) key: Key,
}

/// `OPEN` and `unformatted` distinguish a kept encrypted container from a kept
/// filesystem without adding another action field.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mounted {
    pub(crate) target: String,
    pub(crate) fstype: String,
}

pub(crate) const OPEN: &str = "open";

/// `sfdisk --append` decides a planned partition's number, so its answers stay
/// on the plan entry when a device name moves. An earlier device guess could
/// attach those answers to the wrong partition.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Created {
    pub(crate) gb: u64,
    pub(crate) offset: u64,
    pub(crate) target: String,
    pub(crate) fstype: String,
    pub(crate) label: String,
    /// Only the completed `sfdisk` append knows the assigned device node, so
    /// `cut_partitions` records it after the table changes.
    pub(crate) device: String,
    /// Only the whole-disk plan sets it, because the editor refuses a manual
    /// container.
    pub(crate) encrypt: bool,
    /// An initrd without `root=` needs the cut to type its root partition as
    /// `ROOT_GUID`.
    pub(crate) discoverable: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CustomLayout {
    pub(crate) disk: String,
    pub(crate) mounts: Vec<CustomMount>,
    pub(crate) opens: Vec<LuksOpen>,
    /// Create and delete answers stay separate because `run` applies both
    /// through `sfdisk` before opening any container. A layout that only
    /// assigns and formats therefore writes no partition table.
    pub(crate) deletes: Vec<String>,
    pub(crate) creates: Vec<Created>,
    /// Kept partitions need post-create label writes because their labels are
    /// not part of the append script.
    pub(crate) renames: Vec<Rename>,
    /// Replaced ESP entries must be removed before `bootc` writes the entries
    /// carried by the image.
    pub(crate) esp: Vec<EspRemoval>,
    /// A disk can change after review, so `cut_partitions` compares this state
    /// before applying approved partition numbers to the disk now present.
    /// Any mismatch refuses the write.
    pub(crate) confirmed: Option<DiskState>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Partition {
    pub(crate) device: String,
    pub(crate) size: String,
    pub(crate) fstype: String,
    pub(crate) label: String,
    pub(crate) parttype: String,
    /// The existing crypttab UUID links a container to its discovered system.
    /// Partitions without that link leave this empty.
    pub(crate) uuid: String,
}

impl CustomLayout {
    /// A `Some` layout makes the install manual before any partition answer
    /// exists. `short_of` then refuses the layout until it carries a root.
    /// Incremental state permits the user to enter the manual editor safely.
    pub(crate) fn empty(disk: &str) -> Self {
        Self {
            disk: disk.to_string(),
            ..Default::default()
        }
    }

    /// Re-entering the partition editor must preserve the answer from the
    /// first visit.
    pub(crate) fn answer(&self, partition: &Partition) -> Option<Mounted> {
        self.mounts
            .iter()
            .find(|mount| mount.partition == partition.device)
            .map(|mount| Mounted {
                target: mount.target.clone(),
                fstype: mount.fstype.clone(),
            })
            .or_else(|| {
                self.opens
                    .iter()
                    .find(|open| open.partition == partition.device)
                    .map(|open| Mounted {
                        target: open.target.clone(),
                        fstype: OPEN.to_string(),
                    })
            })
    }

    /// Stable open order gives each container a stable `/dev/mapper` name.
    /// `cryptsetup` needs the bare name, while the installer mounts the path
    /// that `mapper_path` builds from it.
    pub(crate) fn mappers(&self) -> Vec<(String, &LuksOpen)> {
        self.opens
            .iter()
            .enumerate()
            .map(|(at, open)| (format!("tect-{}", at + 1), open))
            .collect()
    }

    pub(crate) fn changes_table(&self) -> bool {
        !(self.deletes.is_empty() && self.creates.is_empty() && self.renames.is_empty())
    }

    /// The root identity matters because a key stored on the root cannot open
    /// that same filesystem.
    pub(crate) fn opens_the_root(&self) -> bool {
        self.opens.iter().any(|open| open.target == "/")
    }

    /// A planned label stands in for the partition's current label until the
    /// disk write applies it.
    pub(crate) fn renamed(&self, partition: &str) -> Option<&str> {
        self.renames
            .iter()
            .find(|rename| rename.partition == partition)
            .map(|rename| rename.label.as_str())
    }

    /// An assignment keeps the filesystem. Only a format answer erases it.
    pub(crate) fn formatted(&self, partition: &str) -> bool {
        self.mounts
            .iter()
            .any(|mount| mount.partition == partition && mount.fstype != "unformatted")
    }

    /// Removing, mounting or opening a linked partition invalidates its ESP
    /// link.
    pub(crate) fn entry_replaced(&self, link: &str) -> bool {
        !link.is_empty()
            && (self.deletes.iter().any(|at| at == link)
                || self.mounts.iter().any(|mount| mount.partition == link)
                || self.opens.iter().any(|open| open.partition == link))
    }

    /// Deleting or rewriting a linked system leaves its ESP entry booting
    /// nothing, so the plan removes that entry before the image writes its own.
    pub(crate) fn entry_gone(&self, link: &str) -> bool {
        !link.is_empty() && (self.deletes.iter().any(|at| at == link) || self.formatted(link))
    }
}

/// LUKS2 leaves gaps after slot removal, so slot indices are read rather than
/// counted.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Slots {
    pub(crate) keys: Vec<u32>,
    pub(crate) tokens: Vec<String>,
}

impl Slots {
    pub(crate) fn has_tpm2(&self) -> bool {
        self.tokens.iter().any(|kind| kind == "systemd-tpm2")
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Opened {
    /// An unchanged container header leaves the key ladder to decide the
    /// machine's access path.
    Keep,
    /// First boot owns TPM2 enrollment for every opened container.
    Tpm2,
    /// A data volume needs a header key file to avoid a passphrase prompt at
    /// every boot.
    AddKey,
}

impl Opened {
    pub(crate) fn of(shown: &str) -> Self {
        match shown {
            copy::OPENED_TPM2 => Self::Tpm2,
            copy::OPENED_ADD_KEY => Self::AddKey,
            _ => Self::Keep,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Keep => copy::OPENED_KEEP,
            Self::Tpm2 => copy::OPENED_TPM2,
            Self::AddKey => copy::OPENED_ADD_KEY,
        }
    }
}

/// Existing key files move into the installed root. Passphrases remain boot
/// prompts, while an added data-volume key permits unattended opening.
pub(crate) fn at_boot(open: &LuksOpen, opened: Opened) -> &'static str {
    if opened == Opened::Tpm2 {
        return copy::BOOT_TPM2;
    }
    // A keyfile on the root cannot open the root, because the file would
    // live on the filesystem the key opens. A root that already has a
    // passphrase asks for it at boot. A keyfile-only root leaves the machine
    // no available key, and `short_of` keeps that answer out of an install.
    if open.target == "/" {
        return match open.key {
            Key::Passphrase(_) => copy::BOOT_PASSPHRASE,
            _ => copy::OPENED_ROOT_KEYFILE,
        };
    }
    match (&open.key, opened) {
        (Key::Passphrase(_), Opened::AddKey) => copy::BOOT_ADDED_KEY,
        (Key::Passphrase(_), _) => copy::BOOT_PASSPHRASE,
        _ => copy::BOOT_KEYFILE,
    }
}

impl Encryption {
    pub(crate) fn wants_passphrase(kind: &str) -> bool {
        kind.ends_with("passphrase")
    }

    pub(crate) fn wants_pin(kind: &str) -> bool {
        kind.ends_with("pin")
    }
}

pub struct Answers {
    pub disk: String,
    pub hostname: String,
    pub user: String,
    pub password: String,
    pub encryption: Encryption,
    pub(crate) layout: Option<CustomLayout>,
    /// An opened container replaces the encryption-kind row with this answer.
    /// Every other install keeps `Keep` because the row still shows the kinds.
    /// The separation prevents a whole-disk kind from becoming a header action.
    pub(crate) opened: Opened,
}

#[derive(Clone, Default)]
pub struct Given {
    pub disk: Option<String>,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub encryption: Option<String>,
    pub passphrase: Option<String>,
    pub pin: Option<String>,
}

/// Every leave choice stays available because the installer asks before it
/// touches a disk.
pub(crate) enum Leave {
    Back,
    Over,
    Shell,
}

/// Ctrl+C leaves a raw-mode widget. Escape cancels a prompt-backed widget.
pub(crate) fn leaving(err: &str) -> bool {
    err == common::ui::INTERRUPTED || common::prompt::cancelled(err)
}

/// Screen coverage reads these choices, so a new leave path cannot bypass its
/// snapshot.
pub(crate) fn leave_options() -> [Choice; 3] {
    [
        Choice::new(copy::LEAVE_BACK, ""),
        Choice::new(copy::LEAVE_OVER, ""),
        Choice::new(copy::LEAVE_SHELL, ""),
    ]
}

pub(crate) fn leave(prompt: &Prompt) -> Result<Leave, String> {
    // A run that draws nothing cannot show the leaving question. Asking it
    // would loop over a question the user never sees.
    if !prompt.draws() {
        return Ok(Leave::Shell);
    }
    let options = leave_options();
    // A leave key pressed on the leaving question returns to the screen it
    // came from.
    match prompt.choose(copy::LEAVING, &options) {
        Ok(Some(1)) => Ok(Leave::Over),
        Ok(Some(2)) => Ok(Leave::Shell),
        Ok(_) => Ok(Leave::Back),
        Err(err) if leaving(&err) => Ok(Leave::Back),
        Err(err) => Err(err),
    }
}
