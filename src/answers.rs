use super::*;

pub struct Encryption {
    pub kind: String,
    pub passphrase: String,
    /// Holds the PIN the user types at every unlock beside the TPM policy.
    /// Only `tpm2-luks-pin` asks for one. `tpm2-luks` leaves it empty,
    /// because that kind's passphrase is a generated recovery key.
    pub pin: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CustomMount {
    pub(crate) partition: String,
    pub(crate) target: String,
    pub(crate) fstype: String,
    /// Holds the passphrase a `luks` format kept. No reader takes it, because
    /// `layout_short_of` refuses a manual `luks` format before the install.
    pub(crate) passphrase: String,
}

/// Holds one partition the plan renames when it cuts the disk. The partition
/// already exists, so the cut writes the name through `sfdisk --part-label`.
/// A planned partition carries its name in the cut script instead.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rename {
    pub(crate) partition: String,
    pub(crate) label: String,
}

/// Holds how the user opens a container. A passphrase is typed here, and a
/// key file is one this live system can read. Either one is secret, so it is
/// never drawn, logged or written into a recipe.
#[derive(Clone, PartialEq)]
pub(crate) enum Key {
    Passphrase(String),
    File(PathBuf),
    /// Holds a key read out of an old system. That key has no path left once
    /// the walk that found it unmounts. `cryptsetup` takes the bytes the way
    /// it takes a passphrase.
    Data(Vec<u8>),
}

impl Key {
    /// Returns what `cryptsetup` reads on stdin. A key file returns nothing,
    /// because it is passed by name and never travels through this process.
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

/// Holds one container the user asked to open and where its filesystem
/// mounts.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LuksOpen {
    pub(crate) partition: String,
    pub(crate) target: String,
    pub(crate) key: Key,
}

/// Holds one partition's answer, which is where it mounts and how it is made
/// ready. An `OPEN` fstype marks a container that is decrypted and kept. An
/// `unformatted` fstype marks a filesystem that is kept as it is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mounted {
    pub(crate) target: String,
    pub(crate) fstype: String,
}

/// The sentinel an answer carries for a container that is opened instead of
/// formatted. It is read out of an answer and never written into a recipe.
pub(crate) const OPEN: &str = "open";

/// Holds a partition the plan will cut before the install formats it. `sfdisk
/// --append` decides the number, so the screens derive the device from the
/// surviving partitions every time. The answers ride on this plan entry
/// rather than on a name that moves.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Created {
    /// Holds the size the user typed, in whole GB.
    pub(crate) gb: u64,
    /// Holds how far into the free region the user placed the partition, in
    /// whole GB. Zero starts the partition at the region's first sector.
    pub(crate) offset: u64,
    /// Names where it mounts. It stays empty until the user assigns it.
    pub(crate) target: String,
    /// Names what it is formatted as. It stays empty until the user chooses
    /// a format.
    pub(crate) fstype: String,
    /// Names the label the cut writes into the partition table. It stays
    /// empty until the user renames the partition.
    pub(crate) label: String,
    /// Names the node the partition actually got. `cut_partitions` writes it
    /// once `sfdisk` has appended the partition, from the node the disk gives
    /// it then. It stays empty until then, because only the cut knows the
    /// node for a fact.
    pub(crate) device: String,
    /// Only the whole-disk plan sets it, because the editor refuses a manual
    /// container.
    pub(crate) encrypt: bool,
    /// Marks a partition the cut types `ROOT_GUID`, so an initrd with no
    /// `root=` argument finds it.
    pub(crate) discoverable: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CustomLayout {
    pub(crate) disk: String,
    pub(crate) mounts: Vec<CustomMount>,
    pub(crate) opens: Vec<LuksOpen>,
    /// Hold the existing partitions the plan removes and the ones it
    /// appends. `run` enacts both through `sfdisk` before it opens any
    /// container. Both stay empty until the user takes a create or clear
    /// menu item, so a layout that only assigns and formats writes no
    /// partition table at all.
    pub(crate) deletes: Vec<String>,
    pub(crate) creates: Vec<Created>,
    /// Holds the labels the plan writes onto partitions it keeps. `run`
    /// enacts each through `sfdisk --part-label` after the creates.
    pub(crate) renames: Vec<Rename>,
    /// Holds the ESP entries the plan replaces. `run` removes them before
    /// `bootc` writes the entries the image carries.
    pub(crate) esp: Vec<EspRemoval>,
    /// Holds the disk the user reviewed immediately before the destructive
    /// summary. `cut_partitions` refuses a replaced or changed table rather
    /// than applying the approved partition numbers to the disk now present.
    pub(crate) confirmed: Option<DiskState>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Partition {
    pub(crate) device: String,
    pub(crate) size: String,
    pub(crate) fstype: String,
    pub(crate) label: String,
    pub(crate) parttype: String,
    /// Carries what an old system's crypttab names the container by. A
    /// partition with no uuid leaves it empty.
    pub(crate) uuid: String,
}

impl CustomLayout {
    /// Builds a manual layout with nothing answered in it yet. A `Some`
    /// layout is what makes the install manual at all. `short_of` refuses an
    /// empty one until it carries a root.
    pub(crate) fn empty(disk: &str) -> Self {
        Self {
            disk: disk.to_string(),
            ..Default::default()
        }
    }

    /// Returns the answer a partition already carries. The editor opened a
    /// second time keeps what the user said the first time.
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

    /// Names each opened container for `/dev/mapper`, in the order the
    /// layout opens them. `cryptsetup` takes the bare name. The installer
    /// mounts the path `mapper_path` builds from it.
    pub(crate) fn mappers(&self) -> Vec<(String, &LuksOpen)> {
        self.opens
            .iter()
            .enumerate()
            .map(|(at, open)| (format!("tect-{}", at + 1), open))
            .collect()
    }

    /// Whether the cut writes the partition table at all.
    pub(crate) fn changes_table(&self) -> bool {
        !(self.deletes.is_empty() && self.creates.is_empty() && self.renames.is_empty())
    }

    /// Whether a container this layout opens is the root, which is the
    /// filesystem every key file for the machine is read from. Six call
    /// sites ask it, so the test lives here once.
    pub(crate) fn opens_the_root(&self) -> bool {
        self.opens.iter().any(|open| open.target == "/")
    }

    /// Gives the label the plan writes onto one partition it keeps, which
    /// stands in for the label the partition carries now.
    pub(crate) fn renamed(&self, partition: &str) -> Option<&str> {
        self.renames
            .iter()
            .find(|rename| rename.partition == partition)
            .map(|rename| rename.label.as_str())
    }

    /// Reports whether the plan rewrites one partition's filesystem. An
    /// assigned partition keeps its filesystem, because only a format answer
    /// erases it.
    pub(crate) fn formatted(&self, partition: &str) -> bool {
        self.mounts
            .iter()
            .any(|mount| mount.partition == partition && mount.fstype != "unformatted")
    }

    /// Reports whether the plan takes the partition one link names, so the
    /// entry that named it is replaced. A mount point, a format and an opened
    /// container each take the partition. A delete removes it.
    pub(crate) fn entry_replaced(&self, link: &str) -> bool {
        !link.is_empty()
            && (self.deletes.iter().any(|at| at == link)
                || self.mounts.iter().any(|mount| mount.partition == link)
                || self.opens.iter().any(|open| open.partition == link))
    }

    /// Reports whether the system one link names is deleted or rewritten,
    /// which leaves its ESP entry booting nothing. The plan removes those
    /// entries and the image writes its own.
    pub(crate) fn entry_gone(&self, link: &str) -> bool {
        !link.is_empty() && (self.deletes.iter().any(|at| at == link) || self.formatted(link))
    }
}

/// Holds what one container's header carries, read through `luksDump`. LUKS2
/// numbers key slots 0 to 31 and leaves a gap where one was removed, so the
/// slot indices are read rather than counted.
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

/// Holds what the user chose on the row that replaces the encryption kinds
/// once the layout opens a container. The row is one field, so it takes one
/// answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Opened {
    /// Keeps what the container's header already holds. The key ladder then
    /// decides the machine's way in.
    Keep,
    /// A first-boot enrollment adds a TPM2 token to every opened container.
    Tpm2,
    /// Adds a key file to a data volume's header, so it stops asking for a
    /// passphrase at every boot.
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

/// Says how the installed machine opens one container it did not re-key.
/// The key ladder runs from the top. The keyfile an old system holds is
/// carried to the new root. A passphrase is asked for at boot. A key added
/// here is what a data volume takes when the user wants it unattended.
pub(crate) fn at_boot(open: &LuksOpen, opened: Opened) -> &'static str {
    if opened == Opened::Tpm2 {
        return copy::BOOT_TPM2;
    }
    // A keyfile on the root cannot open the root, because the file would
    // live on the filesystem the key opens. A root that already has a
    // passphrase asks for it at boot. A keyfile-only root leaves the machine
    // nothing to read, and `short_of` keeps that answer out of an install.
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

/// Holds the user's half of the install. Nothing derives these answers and
/// no flag defaults them.
pub struct Answers {
    pub disk: String,
    pub hostname: String,
    pub user: String,
    pub password: String,
    pub encryption: Encryption,
    pub(crate) layout: Option<CustomLayout>,
    /// Holds what the row replacing the encryption kinds answered, once the
    /// layout opens a container. Every other install keeps `Keep`, because
    /// the row still shows the kinds.
    pub(crate) opened: Opened,
}

/// Holds what the flags gave. The first pass reads it to seed each field. A
/// field asked a second time opens on the answer it already carries.
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

/// Names what a leave key is answered with. All three stay available for as
/// long as no disk has been touched, which is the whole of when the
/// installer asks.
pub(crate) enum Leave {
    /// Returns to the screen the key was pressed on and keeps the answers.
    Back,
    /// Asks the questions again, from the first one.
    Over,
    /// Leaves the installer, having written nothing.
    Shell,
}

/// Whether a widget's error is the user leaving. Esc is the other half of
/// leaving, and it arrives as a `None`.
pub(crate) fn leaving(err: &str) -> bool {
    err == common::ui::INTERRUPTED
}

/// What a leave key asks before it leaves.
pub(crate) fn leave(prompt: &Prompt) -> Result<Leave, String> {
    // A run that draws nothing cannot show the leaving question. Asking it
    // would loop over a question the user never sees.
    if !prompt.draws() {
        return Ok(Leave::Shell);
    }
    let options = [
        Choice::new(copy::LEAVE_BACK, ""),
        Choice::new(copy::LEAVE_OVER, ""),
        Choice::new(copy::LEAVE_SHELL, ""),
    ];
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
