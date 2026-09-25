use super::*;

/// Holds half a kilobyte. `/sys/block/<disk>/size` counts in these units
/// whatever sector size the device itself uses.
const SECTOR: u64 = 512;

/// Names the block-device prefixes the disk list drops. No user installs
/// onto one of these, and each one is only an option to get wrong.
const VIRTUAL: [&str; 7] = ["loop", "ram", "zram", "sr", "fd", "dm-", "md"];

/// Reads what `/proc/mounts` says. One read serves a whole listing, and
/// handing the text in lets a test drive the rule below.
pub(crate) fn in_use_now() -> String {
    std::fs::read_to_string("/proc/mounts").unwrap_or_default()
}

/// Whether the running system is already using this disk.
///
/// On installer media that disk is the medium itself. The medium is a whole
/// disk like any other, and `/sys/block` says nothing about which one was
/// booted, so the list would otherwise offer it beside the machine's own
/// disks and partitioning it would erase the installer. The test asks what is
/// mounted. A stick written with `dd` stays writable, so a read-only test
/// would miss it.
fn in_use(disk: &Path, name: &str, mounts: &str) -> bool {
    let is_source = |dev: &str| {
        mounts
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .any(|source| source == dev)
    };
    if is_source(&format!("/dev/{name}")) {
        return true;
    }
    let Ok(parts) = std::fs::read_dir(disk) else {
        return false;
    };
    parts.flatten().any(|part| {
        part.path().join("partition").is_file()
            && is_source(&format!("/dev/{}", part.file_name().to_string_lossy()))
    })
}

/// Lists the whole disks this machine has, as `/sys/block` holds them. Each
/// one carries what the user needs to tell it from the others. `mounts` is
/// `/proc/mounts`, which keeps the medium this installer runs from out of the
/// list.
pub fn disks(sys: &Path, mounts: &str) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(sys) else {
        return Vec::new();
    };
    let mut found: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if VIRTUAL.iter().any(|prefix| name.starts_with(prefix)) {
            continue;
        }
        let read = |leaf: &str| {
            std::fs::read_to_string(entry.path().join(leaf))
                .map(|text| text.trim().to_string())
                .unwrap_or_default()
        };
        let sectors: u64 = read("size").parse().unwrap_or(0);
        if sectors == 0 {
            continue;
        }
        // A disk nothing can write to is no install target. A disk the
        // running system already uses is the medium this installer runs
        // from.
        if read("ro") == "1" || in_use(&entry.path(), &name, mounts) {
            continue;
        }
        let removable = match read("removable").as_str() {
            "1" => copy::REMOVABLE.to_string(),
            _ => String::new(),
        };
        let gigs = sectors as f64 * SECTOR as f64 / 1_000_000_000.0;
        let detail = [
            copy::size_said(&format!("{gigs:.1}")),
            read("device/model"),
            removable,
        ];
        found.push((
            format!("/dev/{name}"),
            detail
                .iter()
                .filter(|part| !part.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join("  "),
        ));
    }
    found.sort();
    found
}

/// Asks the user which disk to install onto, and defaults to none. A run
/// that cannot ask and holds no earlier answer refuses and names `--disk`. A
/// machine whose `/sys/block` lists nothing takes a typed answer.
pub(crate) fn ask_disk(
    given: Option<String>,
    current: Option<&str>,
    prompt: &Prompt,
) -> Result<String, String> {
    if let Some(disk) = given.filter(|disk| !disk.is_empty()) {
        return Ok(disk);
    }
    let found = disks(&sys_block(), &in_use_now());
    if !prompt.asks() || found.is_empty() {
        return prompt.text(None, copy::INSTALL_DISK, "--disk", current);
    }
    let options: Vec<Choice> = found
        .iter()
        .map(|(disk, detail)| Choice::new(disk, detail))
        .collect();
    let at = current
        .and_then(|held| found.iter().position(|(disk, _)| disk == held))
        .unwrap_or(0);
    match prompt.choose_current(copy::INSTALL_DISK, &options, at)? {
        Some(at) => Ok(found[at].0.clone()),
        // An unanswered question keeps the answer the form already had. The
        // first pass holds none, which is what blocks the action.
        None => Ok(current.unwrap_or_default().to_string()),
    }
}

/// Returns the encryption kind where fisherman has one by that name. The
/// form validates a flag this way without asking the user anything.
pub(crate) fn named(kind: String) -> Result<String, String> {
    if KINDS.iter().any(|(name, _, _)| *name == kind) {
        return Ok(kind);
    }
    Err(format!(
        "`{kind}` is not one of {}",
        KINDS
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Reads a description back as the name fisherman is given, which reverses
/// `shown`. A flag is scripted, so it takes fisherman's names, and the screen
/// shows the descriptions instead. A label no row holds reads as `none`.
pub(crate) fn written(shown: &str) -> &'static str {
    KINDS
        .iter()
        .find(|(_, label, _)| *label == shown)
        .map_or(NONE, |(name, _, _)| *name)
}

/// Returns the description a kind's name is shown as, which the screens read
/// an answer back through.
pub(crate) fn shown(kind: &str) -> &'static str {
    KINDS
        .iter()
        .find(|(name, _, _)| *name == kind)
        .map_or(copy::ENC_NONE, |(_, label, _)| *label)
}

/// Finds a kind's index in the list that was built, by the label the screen
/// shows. The window that makes a container leaves `none` out, so its indices
/// run behind `KINDS` and the answer is found in the built list.
pub(crate) fn kind_at(options: &[Choice], kind: &str) -> Option<usize> {
    options
        .iter()
        .position(|choice| choice.label == shown(kind))
}

/// Builds the encryption kinds. A `tpm2-` kind on a machine with no TPM is
/// hidden from the menu, and validation refuses a seeded answer that asks for
/// it. The window that makes a container from a partition passes `with_none`
/// false, because a LUKS header with no key is no answer.
pub(crate) fn kinds(tpm: bool, luks_initramfs: bool, with_none: bool) -> Vec<Choice> {
    KINDS
        .iter()
        .filter(|(name, _, _)| with_none || *name != NONE)
        .map(|(name, shown, detail)| match () {
            _ if *name != NONE && !luks_initramfs => Choice::new(*shown, copy::NO_LUKS_INITRAMFS)
                .unavailable()
                .hidden(),
            _ if !tpm && name.starts_with("tpm2") => {
                Choice::new(*shown, copy::NO_TPM).unavailable().hidden()
            }
            _ => Choice::new(*shown, *detail),
        })
        .collect()
}

/// The numbered prompt has no form renderer to filter its options, so each
/// visible option keeps the index it holds in `KINDS`.
pub(crate) fn visible_kinds(tpm: bool, luks_initramfs: bool) -> Vec<(usize, Choice)> {
    kinds(tpm, luks_initramfs, true)
        .into_iter()
        .enumerate()
        .filter(|(_, choice)| !choice.hidden)
        .collect()
}

/// Builds the encryption row. The row offers the kinds until the layout
/// opens a container, and what the opened headers hold after that. `held` is
/// the row being replaced, so a layout edited again keeps its answer.
pub(crate) fn encryption_row(
    layout: Option<&CustomLayout>,
    held: &common::ui::Field,
    tpm: bool,
    luks_initramfs: bool,
) -> common::ui::Field {
    match layout.filter(|layout| !layout.opens.is_empty()) {
        Some(layout) => {
            let chosen = Opened::of(&held.value());
            let rows = opened_rows(layout, tpm, luks_initramfs);
            // An answer whose option is gone falls back to keeping the
            // header as it is. A row with nothing chosen reads as `not set`
            // and would carry the empty string into the answer.
            let at = rows
                .iter()
                .position(|row| row.label == chosen.label())
                .or_else(|| rows.iter().position(|row| row.label == copy::OPENED_KEEP));
            common::ui::Field::pick(copy::ROW_ENCRYPTION, rows, at)
        }
        // A whole-disk install keeps the kind on the row. The window that
        // row opens is where the user picks the kind and types its
        // passphrase.
        None => common::ui::Field::action(copy::ROW_ENCRYPTION, shown(written(&held.value()))),
    }
}

/// Builds what the encryption row holds once the layout opens containers.
/// The rows carry the headers' slots and the two additions worth offering.
/// No row re-keys a container or removes a slot, so the first answer is what
/// the key ladder does with the slots already there. A header the walk could
/// not read refuses its additions with that reason rather than aborting the
/// form.
pub(crate) fn opened_rows(layout: &CustomLayout, tpm: bool, luks_initramfs: bool) -> Vec<Choice> {
    let read: Vec<Result<Slots, String>> = layout
        .opens
        .iter()
        .map(|open| slots(&open.partition))
        .collect();
    let said = match read.is_empty() {
        true => String::new(),
        false => read
            .iter()
            .map(|slots| match slots {
                Ok(slots) => copy::slots_said(&slots.keys, &slots.tokens),
                Err(why) => copy::slots_unknown(why),
            })
            .collect::<Vec<_>>()
            .join("; "),
    };
    let mut rows = vec![Choice::new(copy::OPENED_KEEP, said)];
    // A root a key file opens cannot keep that arrangement. The machine has
    // nothing to read at boot, and a token staged inside the root cannot be
    // enrolled before the first boot unlocks it. The reason sits on both
    // answers, so neither looks like a way through.
    let keyfile_root = layout
        .opens
        .iter()
        .any(|open| open.target == "/" && !matches!(open.key, Key::Passphrase(_)));
    if keyfile_root {
        rows[0] = Choice::new(copy::OPENED_KEEP, copy::OPENED_ROOT_KEYFILE)
            .unavailable()
            .hidden();
    }
    let unread = read.iter().find_map(|slots| slots.as_ref().err());
    let enrolled = read
        .iter()
        .any(|slots| slots.as_ref().is_ok_and(Slots::has_tpm2));
    // The first-boot enrolment stages the key that unlocks the container
    // beside its unit, so an unencrypted root refuses it for the same reason
    // it refuses a key file. The layout settles that without reading a
    // header, so this arm stands before the arms that read one.
    let encrypted_root = layout.opens_the_root();
    rows.push(match (tpm, unread, enrolled) {
        (false, _, _) => Choice::new(copy::OPENED_TPM2, copy::NO_TPM)
            .unavailable()
            .hidden(),
        _ if !encrypted_root => Choice::new(copy::OPENED_TPM2, copy::OPENED_KEYFILE_PLAIN)
            .unavailable()
            .hidden(),
        (_, Some(why), _) => Choice::new(copy::OPENED_TPM2, why.clone())
            .unavailable()
            .hidden(),
        (_, _, true) => Choice::new(copy::OPENED_TPM2, copy::OPENED_TPM2_HAS)
            .unavailable()
            .hidden(),
        _ if keyfile_root => Choice::new(copy::OPENED_TPM2, copy::OPENED_ROOT_KEYFILE)
            .unavailable()
            .hidden(),
        _ => Choice::new(copy::OPENED_TPM2, copy::OPENED_TPM2_COST),
    });
    // A key file serves a data volume alone. On a root it would live on the
    // filesystem its own key opens, so a root asks for a passphrase or takes
    // a token. The row appears only where the editor holds a passphrase for
    // the volume, which is the rung above that cannot open the machine
    // without the user at the console. It appears only where the root is an
    // opened container too, because a key file on an unencrypted root would
    // be readable beside the volume it opens.
    if let Some((_, slots)) = layout
        .opens
        .iter()
        .zip(read.iter())
        .find(|(open, _)| open.target != "/" && matches!(open.key, Key::Passphrase(_)))
    {
        // The count is the one before the addition. The key added here
        // takes one more slot, and the last screen gives the count after.
        let detail = match slots {
            Ok(slots) => format!(
                "{}; {} slots now",
                copy::OPENED_ADD_KEY_COST,
                slots.keys.len()
            ),
            Err(_) => copy::OPENED_ADD_KEY_COST.to_string(),
        };
        rows.push(match encrypted_root {
            true => Choice::new(copy::OPENED_ADD_KEY, detail),
            false => Choice::new(copy::OPENED_ADD_KEY, copy::OPENED_KEYFILE_PLAIN)
                .unavailable()
                .hidden(),
        });
    }
    if layout.opens_the_root() && !luks_initramfs {
        for row in &mut rows {
            *row = Choice::new(row.label.clone(), copy::NO_LUKS_INITRAMFS)
                .unavailable()
                .hidden();
        }
    }
    rows
}

/// Reads what one container's header holds. `--dump-json-metadata` is LUKS2
/// only, so a container `cryptsetup` cannot read returns a reason for the
/// row. The form keeps running.
pub(crate) fn slots(container: &str) -> Result<Slots, String> {
    let out = Command::new("cryptsetup")
        .args(["luksDump", "--dump-json-metadata"])
        .arg(container)
        .output()
        .map_err(|err| format!("cryptsetup: {err}, and it is what reads a header"))?;
    match out.status.success() {
        true => slots_from(&String::from_utf8_lossy(&out.stdout)),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// Reads the keyslot indices and the token types a `luksDump` document
/// names. LUKS2 stops at slot 31, which is the range this walks.
pub(crate) fn slots_from(raw: &str) -> Result<Slots, String> {
    const SLOTS: u32 = 32;
    let doc = Json::parse(raw).map_err(|err| format!("luksDump wrote invalid JSON: {err}"))?;
    let mut keys = Vec::new();
    if let Some(keyslots) = json::field(&doc, "keyslots") {
        for at in 0..SLOTS {
            if json::field(keyslots, &at.to_string()).is_some() {
                keys.push(at);
            }
        }
    }
    let mut tokens = Vec::new();
    if let Some(listed) = json::field(&doc, "tokens") {
        let Json::Object(entries) = listed else {
            return Err("luksDump wrote tokens that are not an object".to_string());
        };
        for (_, token) in entries {
            if let Some(kind) = json::text(token, "type") {
                tokens.push(kind);
            }
        }
    }
    tokens.sort();
    Ok(Slots { keys, tokens })
}

/// Builds what the `home and data` row offers. One answer keeps the system
/// and the home on one partition. The other cuts a home partition of its own
/// out of the install disk, and that partition is `/var` itself. A composefs
/// target binds `/var` from the root before any fstab unit runs, so the
/// separate answer is hidden there rather than offered and never mounted.
pub(crate) fn data_rows(composefs: bool) -> Vec<Choice> {
    let separate = Choice::new(copy::DATA_SEPARATE, "");
    vec![
        Choice::new(copy::DATA_TOGETHER, ""),
        match composefs {
            true => separate.hidden(),
            false => separate,
        },
    ]
}

/// Reads the `home and data` answer back off its own label. Only the
/// separate answer carries a size.
pub(crate) fn chose(shown: &str, size: &str) -> Data {
    match shown == copy::DATA_SEPARATE {
        true => Data {
            size: size.to_string(),
        },
        false => Data::default(),
    }
}

/// Says what the summary makes of the `home and data` answer. The home row
/// under it carries the size, which is the one place the partition is spelled
/// out.
pub(crate) fn data_said(data: &Data) -> String {
    match data.size.is_empty() {
        true => copy::DATA_TOGETHER.to_string(),
        false => copy::DATA_SEPARATE.to_string(),
    }
}

/// Asks for the encryption where no form draws. A `tpm2-` kind on a machine
/// with no TPM is left out of the menu, and a flag that supplies one is still
/// refused. A run with no window would otherwise carry a kind the machine
/// cannot use into the recipe.
pub(crate) fn ask_encryption(
    given: Option<String>,
    passphrase: Option<String>,
    pin: Option<String>,
    prompt: &Prompt,
    tpm: bool,
    luks_initramfs: bool,
) -> Result<Encryption, String> {
    let kind = match given {
        Some(kind) => named(kind)?,
        None if !prompt.asks() => NONE.to_string(),
        None => {
            let options = visible_kinds(tpm, luks_initramfs);
            let shown: Vec<Choice> = options.iter().map(|(_, choice)| choice.clone()).collect();
            match prompt.choose_current(copy::INSTALL_ENCRYPTION, &shown, 0)? {
                Some(at) => KINDS[options[at].0].0.to_string(),
                None => NONE.to_string(),
            }
        }
    };
    if kind.starts_with("tpm2") && !tpm {
        return Err(copy::NO_TPM.to_string());
    }
    if kind != NONE && !luks_initramfs {
        return Err(copy::NO_LUKS_INITRAMFS.to_string());
    }
    Ok(Encryption {
        passphrase: match Encryption::wants_passphrase(&kind) {
            true => prompt.text(passphrase, copy::LUKS_PASSPHRASE, "--passphrase", None)?,
            false => String::new(),
        },
        pin: match Encryption::wants_pin(&kind) {
            true => prompt.text(pin, copy::LUKS_PIN, "--pin", None)?,
            false => String::new(),
        },
        kind,
    })
}
