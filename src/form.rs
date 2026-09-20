use super::*;

/// Numbers the form rows, so `short_of` and `Answers::of` name the row each
/// one reads. `common::ui::form` draws the `OS setup` heading over the first
/// four rows and the `Disk setup` heading from the layout row down.
pub(crate) const ROW_HOSTNAME: usize = 0;
pub(crate) const ROW_ACCOUNT: usize = 1;
pub(crate) const ROW_PASSWORD: usize = 2;
pub(crate) const ROW_CONFIRM: usize = 3;
/// Leads the disk section, because the layout answer decides which rows
/// under it the form asks.
pub(crate) const ROW_LAYOUT: usize = 4;
/// Answers what opens the root. A manual layout that opens containers
/// replaces this row with the per-container answer.
pub(crate) const ROW_ENCRYPTION: usize = 5;
/// Asks whether the home area gets a partition of its own. The size row
/// under it is asked only when the user answers yes.
pub(crate) const ROW_DATA: usize = 6;
pub(crate) const ROW_SIZE: usize = 7;
/// Draws the disk table under the answers above it. `chosen_disk` reads the
/// chosen disk row back from this field.
pub(crate) const ROW_TABLE: usize = 8;
/// Stays out of the drawn rows, because the encryption window writes it.
pub(crate) const ROW_PASSPHRASE: usize = 9;
/// Stays out of the drawn rows as well. `Answers::of` reads it back only for
/// a `tpm2-luks-pin` install.
pub(crate) const ROW_PIN: usize = 10;

/// Returns the layout the form's layout row says is held. A `manual` answer
/// always holds one for this disk, even with nothing answered in it. A
/// whole-disk answer holds none. The row is read from the form field because
/// the form changes it while the calling command still holds the old layout.
pub(crate) fn held_pick(
    fields: &[common::ui::Field],
    layout: Option<&CustomLayout>,
    disk: &str,
) -> Option<CustomLayout> {
    match fields[ROW_LAYOUT].value() == copy::LAYOUT_MANUAL {
        true => Some(match layout {
            Some(held) if held.disk == disk => held.clone(),
            _ => CustomLayout::empty(disk),
        }),
        false => None,
    }
}

/// Lists the rows the form asks. The encryption window owns the passphrase
/// and the PIN, so neither row is asked here. A manual layout answers its own
/// encryption and home rows, so the whole-disk rows drop out of the list. The
/// size row is asked only when the user answers a separate home partition.
pub(crate) fn asked(fields: &[common::ui::Field]) -> Vec<usize> {
    let manual = fields[ROW_LAYOUT].value() == copy::LAYOUT_MANUAL;
    // A manual layout whose containers are open keeps its encryption row.
    // The row then holds what the container headers answer.
    let opens = matches!(fields[ROW_ENCRYPTION], common::ui::Field::Pick { .. });
    let sized = fields[ROW_DATA].value() == copy::DATA_SEPARATE;
    (0..fields.len())
        .filter(|row| *row != ROW_PASSPHRASE && *row != ROW_PIN)
        .filter(|row| {
            !manual
                || (*row != ROW_ENCRYPTION && *row != ROW_DATA && *row != ROW_SIZE)
                || (opens && *row == ROW_ENCRYPTION)
        })
        .filter(|row| manual || *row != ROW_SIZE || sized)
        .collect()
}

/// Returns both rows of a pair whose two answers disagree. An empty half is a
/// missing answer, so the form calls a pair wrong only once the user has
/// filled both halves.
pub(crate) fn pair_mismatch(fields: &[common::ui::Field], pair: [usize; 2]) -> Vec<usize> {
    let (left, right) = (fields[pair[0]].value(), fields[pair[1]].value());
    match left.is_empty() || right.is_empty() || left == right {
        true => Vec::new(),
        false => pair.to_vec(),
    }
}

/// Says what the form is still short of, which is what the `Install` action
/// shows while it stays unpickable. It covers the values nothing derives and
/// no default fills. It also covers the two password halves agreeing.
pub(crate) fn short_of(
    fields: &[common::ui::Field],
    disk: &str,
    layout: Option<&CustomLayout>,
    payload: &Payload,
    tpm: bool,
    table: Option<&DiskTable>,
) -> Option<String> {
    let at = |row: usize| fields[row].value();
    let held = held_pick(fields, layout, disk);
    let manual = held.is_some();
    // The form already draws a disagreeing password pair red. An empty answer
    // here blocks the action without repeating what the red already says.
    if !pair_mismatch(fields, [ROW_PASSWORD, ROW_CONFIRM]).is_empty() {
        return Some(String::new());
    }
    if let Some(layout) = held.as_ref() {
        // `held_pick` hands back this disk's layout or a fresh one, so no
        // answer here belongs to another disk.
        if let Some(short) = layout_short_of(layout, payload.composefs, table, payload.reserve_gb())
        {
            return Some(short);
        }
        if layout.opens_the_root() && !payload.luks_initramfs {
            return Some(copy::NO_LUKS_INITRAMFS.to_string());
        }
        // A root opened by a key file leaves the machine nothing to read at
        // boot. No answer on the encryption row gives it one.
        if layout
            .opens
            .iter()
            .any(|open| open.target == "/" && !matches!(open.key, Key::Passphrase(_)))
        {
            return Some(copy::OPENED_ROOT_KEYFILE.to_string());
        }
        // A key addition needs an encrypted root. A TPM2 token staged beside
        // a plain root would sit readable, and a key file on a data volume
        // would have nowhere to be read from. Turning the root back to a
        // plain mount refuses here, before fisherman writes the disk.
        if !layout.opens_the_root() && Opened::of(&at(ROW_ENCRYPTION)) != Opened::Keep {
            return Some(copy::OPENED_KEYFILE_PLAIN.to_string());
        }
        // A container that opens something other than the root still needs a
        // key this machine can read at boot, which a plain root denies it.
        if !layout.opens_the_root()
            && layout
                .opens
                .iter()
                .any(|open| open.target != "/" && !matches!(open.key, Key::Passphrase(_)))
        {
            return Some(copy::OPENED_KEYFILE_PLAIN.to_string());
        }
    }
    // A manual layout creates no encryption of its own, so the whole-disk
    // kinds never answer it, however the hidden row was left.
    let kind = match manual {
        true => NONE,
        false => written(&at(ROW_ENCRYPTION)),
    };
    if kind != NONE && !payload.luks_initramfs {
        return Some(copy::NO_LUKS_INITRAMFS.to_string());
    }
    // An `--encryption` flag can seed a `tpm2-` kind this machine cannot
    // enrol, without opening the window that would draw the kind unavailable.
    if kind.starts_with("tpm2") && !tpm {
        return Some(copy::NO_TPM.to_string());
    }
    // A home size the disk cannot hold is refused here, before sfdisk
    // repartitions the disk. The room is measured against the plan's own disk
    // row, less the 2 GB ESP, the 2 GB `/boot` a GRUB target wants and the
    // root's reserve. The refusal tells the user to keep home on the root.
    if !manual && at(ROW_DATA) == copy::DATA_SEPARATE && !at(ROW_SIZE).is_empty() {
        if let (Some((disk, disk_gb)), Some(home)) = (chosen_disk(fields), size_gb(&at(ROW_SIZE))) {
            if home < 1 {
                return Some(copy::home_too_small(&copy::size_said(&at(ROW_SIZE))));
            }
            let boot = u64::from(payload.bootloader != "systemd") * 2;
            let room = disk_gb.saturating_sub(2 + boot + payload.reserve_gb());
            if home > room {
                return Some(copy::home_too_big(&copy::size_said(&at(ROW_SIZE)), &disk));
            }
        }
    }
    let wants = Encryption::wants_passphrase(kind);
    let wants_pin = Encryption::wants_pin(kind);
    let missing: Vec<&str> = [
        (at(ROW_HOSTNAME).is_empty(), copy::ROW_HOSTNAME),
        (disk.is_empty(), copy::INSTALLATION_DISK),
        (at(ROW_ACCOUNT).is_empty(), copy::ROW_ACCOUNT),
        (at(ROW_PASSWORD).is_empty(), copy::ROW_PASSWORD),
        // An empty confirmation is a missing answer of its own.
        (
            !at(ROW_PASSWORD).is_empty() && at(ROW_CONFIRM).is_empty(),
            copy::ROW_CONFIRM,
        ),
        (wants && at(ROW_PASSPHRASE).is_empty(), copy::ROW_PASSPHRASE),
        // An `--encryption tpm2-luks-pin` flag with no `--pin` seeds a drawn
        // form, which never opens the window that would otherwise refuse it.
        (wants_pin && at(ROW_PIN).is_empty(), copy::ROW_PIN),
        (
            !manual && at(ROW_DATA) == copy::DATA_SEPARATE && at(ROW_SIZE).is_empty(),
            copy::ROW_SIZE,
        ),
    ]
    .into_iter()
    .filter(|(missing, _)| *missing)
    .map(|(_, name)| name)
    .collect();
    match missing.is_empty() {
        true => None,
        false => Some(copy::still_needs(&missing)),
    }
}

/// Reads back the disk row the user chose, as the size the plan table draws
/// and as that size in whole GB. It returns `None` before the user chooses a
/// disk, and `None` where the disk field is not a table.
fn chosen_disk(fields: &[common::ui::Field]) -> Option<(String, u64)> {
    let common::ui::Field::Table { rows, .. } = &fields[ROW_TABLE] else {
        return None;
    };
    let row = rows
        .iter()
        .find(|row| row.first().is_some_and(|cell| cell.answered()))?;
    let size = row.get(1)?;
    Some((size.text().to_string(), size_gb(size.text())?))
}
