//! Every string this installer says to the user, in one place.
//!
//! This catalogue holds the questions, the answer labels beside them, the
//! hints under the widgets and the lines an install run prints. The image
//! describes itself in the recipe that carries it. Errors and diagnostics
//! stay with the code that raises them.

// The installer asks the user for these five answers. Nothing on the machine
// derives them. The payload carries the image and the boot chain instead.

pub const INSTALL_DISK: &str = "Installation disk";
pub const INSTALL_NAME: &str = "Hostname";
pub const INSTALL_USER: &str = "Username";
pub const INSTALL_PASSWORD: &str = "Password";
pub const INSTALL_ENCRYPTION: &str = "Encryption type";
pub const LUKS_PASSPHRASE: &str = "LUKS passphrase";
pub const LUKS_PIN: &str = "LUKS PIN";
/// The encryption row shows these labels. `env::KINDS` pairs each one with
/// the name the `--encryption` flag takes.
pub const ENC_NONE: &str = "none";
/// Each label names what opens the container and what the recovery answer
/// is. The encryption row and its list are only a few words wide.
pub const ENC_TPM2: &str = "LUKS with TPM2 + recovery key";
pub const ENC_PASSPHRASE: &str = "LUKS with passphrase";
pub const ENC_BOTH: &str = "LUKS with TPM2 + recovery passphrase";
/// The user types this PIN at every unlock, on top of the TPM policy. A
/// thief who takes the machine gets no automatic TPM unlock. A thief who
/// takes the disk alone gets no passphrase to attack offline.
pub const ENC_TPM2_PIN: &str = "LUKS with TPM2 + PIN";
pub const NO_TPM: &str = "No TPM available";
pub const NO_LUKS_INITRAMFS: &str = "the image has not declared and proved `luks-initramfs`";
pub const REMOVABLE: &str = "removable";

/// The whole-disk plan formats the root with the recipe's `filesystem`, so a
/// recipe without one has no root to write.
pub const NO_ROOT_FILESYSTEM: &str =
    "the recipe names no `filesystem`, and the whole-disk layout formats the root with it";

pub fn unknown_bootloader(bootloader: &str) -> String {
    format!("the recipe's `bootloader` is {bootloader:?}; the installer lays out a disk for `grub2` or `systemd`")
}

pub fn unformattable(fstype: &str, target: &str) -> String {
    format!("{target} cannot be formatted as {fstype:?}; the installer writes btrfs, ext4, xfs and fat32")
}

pub const CUSTOM_ROOT: &str = "still needs a / partition";
/// Bootc installs its bootloader through the ESP. The installer cuts an ESP
/// only on the whole-disk path, so a manual layout must supply one.
pub const CUSTOM_ESP: &str = "still needs a /boot/efi partition";
/// bootc installs only onto an empty root, so a kept `/` would fail after the
/// partition table was cut.
pub const CUSTOM_ROOT_KEPT: &str = "the / partition must be formatted";
/// The installer creates no container from a manual `luks` format. The editor
/// refuses the format here, because `mkfs_args` would otherwise refuse it after
/// the user had confirmed the wipe.
pub const CUSTOM_LUKS_LATER: &str = "manual LUKS is not installed yet; use the whole disk";
/// A composefs deployment is sealed with fs-verity. Only ext4 and btrfs
/// carry fs-verity, so a manual root on xfs or ext3 cannot boot the image.
pub const CUSTOM_VERITY: &str = "the image needs fs-verity: format the root ext4 or btrfs";
/// The missing list names the install disk by what it is. `ROW_DISK` labels
/// the form row that asks for it.
pub const INSTALLATION_DISK: &str = "installation disk";
pub const CUSTOM_DUPLICATE: &str = "each mount point can be used only once";
pub const CUSTOM_UNMOUNTED: &str = "a partition is answered with no mount point";
/// A created partition is cut blank. It has no existing filesystem to keep,
/// so the user must choose what it becomes.
pub const CUSTOM_UNFORMATTED: &str = "a new partition has no filesystem chosen";
/// Every failure after the first disk write says this. The user has to know
/// that the install stopped and that the disk now needs recovering.
pub fn table_already_changed(disk: &str) -> String {
    format!("the partition table on {disk} has already been changed")
}

pub fn no_mkfs(program: &str, fstype: &str) -> String {
    format!("this live environment has no {program}, so it cannot format {fstype}")
}

pub fn root_type_needs_gpt(disk: &str) -> String {
    format!(
        "{disk} needs a GPT partition table, because this image finds its root by partition type"
    )
}

pub fn account_name(user: &str) -> String {
    format!("the username {user:?} needs lowercase letters, digits, `_` or `-`, starting with a letter or `_`")
}

pub fn table_changed(disk: &str) -> String {
    format!("the partition table on {disk} changed while it was being reviewed; review it again")
}

/// `sfdisk --append` appends only to a GPT disk. Clearing the disk writes a
/// fresh GPT label, so clearing is the way forward.
pub fn custom_not_gpt(label: &str) -> String {
    format!("this disk has a {label} partition table: clear it to use GPT")
}

/// One ESP entry the plan removes is not a name this installer may join to a
/// path. The name comes from a directory listing, so a name that is not one
/// path element is a defect rather than a disk state.
pub fn esp_entry_name(entry: &str) -> String {
    format!("`{entry}` is not one entry name under EFI")
}

/// One ESP entry the plan removes is gone from the ESP the walk read, so the
/// disk changed while the plan was reviewed.
pub fn esp_entry_missing(path: &str, why: &str) -> String {
    format!("{path} is not on the ESP: {why}")
}

pub fn esp_entry_remove(path: &str, why: &str) -> String {
    format!("removing {path}: {why}")
}
/// A create takes one free region of the disk, and the room a refusal states
/// is the largest region the plan could not use. The form refuses the size
/// before the confirmation, because the deletes are written before the
/// creates and a later refusal would leave the disk wiped.
pub fn custom_too_big(gb: u64) -> String {
    match gb {
        0 => "no room left on the disk for a new partition".to_string(),
        gb => format!("only {gb} GB is free in the largest free region"),
    }
}

/// The confirmation names one kept container and how the installed machine
/// opens it at boot. A container reports no filesystem until it is open, so
/// the summary says these two facts alone.
pub fn opened(partition: &str, how: &str) -> String {
    format!("{partition}  {how}")
}

/// The confirmation names an opened root's format too, because the install
/// erases the old system inside the container it keeps.
pub fn opened_root(partition: &str, fstype: &str, how: &str) -> String {
    format!("{partition}  format as {fstype} inside, {how}")
}

// The installer never re-keys a container the editor opened. The encryption
// row therefore reports what each LUKS header already holds. The only
// question left for the user is whether to add a token or a key file.

pub const OPENED_KEEP: &str = "keep what is enrolled";
pub const OPENED_TPM2: &str = "add a TPM2 token";
pub const OPENED_ADD_KEY: &str = "add a key file for boot";
pub const OPENED_TPM2_COST: &str = "first boot asks once, then no prompt";
pub const OPENED_TPM2_HAS: &str = "this container already has one";
pub const OPENED_ADD_KEY_COST: &str = "this machine reads it, the old one keeps its own";
/// The installed root holds a data volume's key file. It also holds the key
/// that a first-boot TPM2 enrolment is staged with. An unencrypted root
/// would leave that key in the clear beside the volume it opens, so the
/// ladder falls to the passphrase.
pub const OPENED_KEYFILE_PLAIN: &str = "the root is not encrypted, so the key would be readable";
/// A key file for the root would have to live on the filesystem its own key
/// opens. A token staged inside the root cannot be enrolled until the first
/// boot has already unlocked it.
pub const OPENED_ROOT_KEYFILE: &str = "a key file cannot open the root; use its passphrase";

pub fn slots_said(keys: &[u32], tokens: &[String]) -> String {
    let said: Vec<String> = keys
        .iter()
        .map(|at| format!("slot {at}"))
        .chain(tokens.iter().map(|kind| format!("token {kind}")))
        .collect();
    match said.is_empty() {
        true => "no slots".to_string(),
        false => said.join(", "),
    }
}

pub fn slots_unknown(why: &str) -> String {
    format!("its slots could not be read: {why}")
}

/// Each string names one rung of the unlock ladder. Every one of them
/// repeats `decrypted`, because the summary column is a few words wide and a
/// bare "passphrase at boot" reads as the cost of installing.
pub const BOOT_KEYFILE: &str = "decrypted, keyfile at boot";
pub const BOOT_PASSPHRASE: &str = "decrypted, passphrase at boot";
pub const BOOT_TPM2: &str = "decrypted, TPM2 at boot";
pub const BOOT_ADDED_KEY: &str = "decrypted, key added here";

/// The last screen tells the user which slot the machine no longer needs.
/// The installer never removes a slot itself, because a LUKS header does not
/// record which slot holds what and the old system may still open the volume
/// with it. Each string is one row, because the last screen draws a row as a
/// line.
pub fn kill_slot(device: &str, slot: u32, count: usize) -> Vec<String> {
    vec![
        format!("{device} has {count} slots, and this machine no longer needs slot {slot}"),
        format!("both keys open it, so it can go: cryptsetup luksKillSlot {device} {slot}"),
    ]
}

pub fn kill_a_slot(device: &str, count: usize) -> Vec<String> {
    vec![
        format!("{device} has {count} slots, and this machine no longer needs one of them"),
        format!("cryptsetup luksKillSlot {device} <slot>"),
    ]
}

/// `none` in the third crypttab field tells the installed system to ask the
/// user for a passphrase at boot.
pub fn crypttab_line(name: &str, uuid: &str, keyfile: Option<&str>) -> String {
    format!("{name} UUID={uuid} {} luks", keyfile.unwrap_or("none"))
}

/// The first-boot enrolment prints this before it runs, so the user knows
/// why the console has stopped for a few seconds.
pub const ENROLLING: &str = "enrolling disk auto-unlock; this takes a few seconds";

/// Names the GPT type every EFI System Partition carries. The first-boot
/// unit scans for it, because the installed system mounts no boot partition.
pub const ESP_TYPE: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";

/// `systemd-cryptenroll` seals against the PCRs of the machine it runs on.
/// The live installer's PCRs are not the installed machine's, so this unit
/// enrols on the first boot. The key that opens the container and an optional
/// PIN are staged beside the unit and shredded once the token is in.
///
/// If `pcr_policy` is true, then the installed image carries a signed PCR 11
/// policy and the token binds to that policy as well as to PCR 7.
/// `systemd-stub` places the booted UKI's `.pcrsig` and `.pcrpkey` under
/// `/run/systemd/` as it boots. The unit names both paths outright, because
/// `systemd-cryptenroll` otherwise binds to no PCRs at all when it finds no
/// public key and no signature. A machine that silently seals to nothing is
/// worse than one that fails loudly. An image with no policy keeps PCR 7
/// alone, as every other boot chain does.
///
/// The automatic finalize leaves a marker beside the credential on the ESP,
/// naming the one-time slot it added. The unit then wipes that slot in the
/// same enrolment command and deletes both files, so the credential stops
/// opening the volume at the boot the token arrives. The installed system
/// mounts no boot partition, so the unit finds the ESP by its GPT type. A
/// cleanup that fails holds the login stack for this boot and runs again on
/// the next one, because the key survives a failed exit.
///
/// `Before=systemd-user-sessions.service` holds every getty and display
/// manager until the unit exits. The user then never sees enrolment output
/// printed over a login prompt. The directive orders only, so a failed
/// enrolment releases the login stack as a successful one does.
///
/// `TimeoutStartSec` covers a hang as well as a failure. systemd.service(5)
/// disables the start timeout by default for `Type=oneshot`. Without the
/// timeout, a `systemd-cryptenroll` that blocks on a TPM answering no
/// command would hold `systemd-user-sessions` forever. The key file survives
/// a hang and `ConditionPathExists` stays true, so every later boot would
/// lose its getty and its display manager too.
pub fn tpm2_unit(name: &str, key: &str, pin: Option<&str>, uuid: &str, pcr_policy: bool) -> String {
    let mut enroll = match pcr_policy {
        true => "--tpm2-pcrs=7 --tpm2-public-key=/run/systemd/tpm2-pcr-public-key.pem \
                 --tpm2-signature=/run/systemd/tpm2-pcr-signature.json"
            .to_string(),
        false => "--tpm2-pcrs=7".to_string(),
    };
    if pin.is_some() {
        enroll.push_str(" --tpm2-with-pin=yes");
    }
    let pin_credential = pin
        .map(|path| format!("LoadCredential=cryptenroll.new-tpm2-pin:{path}\n"))
        .unwrap_or_default();
    let shred_pin = pin
        .map(|path| format!("ExecStartPost=-/usr/bin/shred -u {path}\n"))
        .unwrap_or_default();
    let credential = format!("{}/{}.cred", crate::CREDENTIAL_DIR, crate::CREDENTIAL);
    let marker = format!("{}/{}", crate::CREDENTIAL_DIR, crate::SLOT_FILE);
    let script = format!(
        "mnt=/run/tpm2-enroll-esp; slot=; esp=; \
         while read -r name parttype; do \
           [ \"$parttype\" = \"{ESP_TYPE}\" ] || continue; \
           mkdir -p \"$mnt\"; \
           mount \"/dev/$name\" \"$mnt\" 2>/dev/null || continue; \
           if [ -s \"$mnt/{marker}\" ]; then \
             slot=$(cat \"$mnt/{marker}\"); esp=$name; break; \
           fi; \
           umount \"$mnt\" 2>/dev/null; \
         done < <(lsblk -rno NAME,PARTTYPE); \
         wipe=; \
         if [ -n \"$slot\" ] && cryptsetup luksDump \"/dev/disk/by-uuid/{uuid}\" | grep -qE \"^[[:space:]]*$slot: luks2\"; then \
           wipe=\"--wipe-slot=$slot\"; \
         fi; \
         for i in 1 2 3 4 5; do \
           if /usr/bin/systemd-cryptenroll --tpm2-device=auto {enroll} \
              --unlock-key-file={key} $wipe /dev/disk/by-uuid/{uuid}; then \
             if [ -n \"$esp\" ]; then \
               rm -f \"$mnt/{credential}\" \"$mnt/{marker}\" || exit 1; \
               umount \"$mnt\" || exit 1; \
             fi; \
             exit 0; \
           fi; \
           sleep 5; \
         done; \
         exit 1"
    );
    format!(
        "[Unit]\n\
         Description=Enroll the {name} container for TPM2 unlock\n\
         ConditionPathExists={key}\n\
         After=basic.target\n\
         Before=systemd-user-sessions.service\n\
         DefaultDependencies=no\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         RemainAfterExit=no\n\
         TimeoutStartSec=120\n\
         StandardOutput=journal+console\n\
         StandardError=journal+console\n\
         {pin_credential}\
         ExecStartPre=/bin/echo '{ENROLLING}'\n\
         ExecStart=/bin/bash -c '{script}'\n\
         ExecStartPost=-/usr/bin/shred -u {key}\n\
         {shred_pin}\
         ExecStartPost=-/usr/bin/systemctl disable tect-tpm2-enroll-{name}.service\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    )
}

/// The installer verifies every key it adds. A key that does not open its
/// own container is a change the install must not leave behind.
pub fn added_key_wrong(device: &str) -> String {
    format!("the key just added to {device} does not open it")
}

pub const KEY_TYPE: &str = "Key type";
pub const KEY_OPEN: &str = "Open";
pub const KEY_BACK: &str = "Back";
/// The passphrase window takes Enter as its own action, so it draws no
/// button.
pub const PARTITION_ENCRYPTION: &str = "partition encryption";
pub const CONFIRM_PASSPHRASE: &str = "confirm passphrase";
pub const CONFIRM_PIN: &str = "confirm PIN";
pub const LUKS_PARTITION: &str = "luks partition";
pub const ENCRYPTION_TYPE: &str = "encryption type";
pub const SUBMIT_KEYS: &str = "\u{23ce}  submit \u{2022} Esc back \u{2022} Alt+S show";
pub const KEY_PASSPHRASE: &str = "passphrase";
pub const KEY_FILE: &str = "key file";
pub const KEY_FILE_PATH: &str = "key file path";
pub const KEY_FILE_MISSING: &str = "there is no file at that path";
pub const ENCRYPTED_DISK: &str = "encrypted disk";
pub const ENCRYPTED_PARTITION: &str = "encrypted partition";

// Each line names one step of the old-system walk that could not run. A
// silent failure would tell the user their disk needs a new key when it does
// not.

pub const OLD_ROOT_NONE: &str = "no partition on this disk reads as an old system";
pub fn old_root_unnamed(partition: &str) -> String {
    format!("no old system on this disk names a key for {partition}")
}
pub fn old_root_partly(unread: &str) -> String {
    format!("{unread}, and nothing readable named a key")
}
pub fn old_root_key_missing(volume: &str, at: &str, why: &str) -> String {
    format!("the key {at} that opens {volume} could not be read: {why}")
}
pub fn old_root_key_outside(at: &str) -> String {
    format!("the key {at} is not a file inside the system that names it")
}
pub fn old_root_key_wrong(partition: &str) -> String {
    format!("the key the old system named for {partition} does not open it")
}
pub fn old_root_key_unreadable(volume: &str) -> String {
    format!("{volume} opens with a script this installer cannot run")
}
/// The old system reads this volume's key from removable media, through
/// Debian's `passdev` or a `path:device` third crypttab field. The installer
/// refuses the volume instead of copying the key onto the root, because the
/// user keeps that media apart from the machine on purpose.
pub fn old_root_key_removable(volume: &str) -> String {
    format!("{volume} opens with a key file on removable media, which this installer does not copy onto the machine")
}

/// `keyfile-timeout=` waits for a key file to appear at boot, which is the
/// same removable-media arrangement said another way. The installer does not
/// configure it.
pub fn old_root_key_waited(volume: &str) -> String {
    format!("{volume} waits for its key file at boot, which this installer does not configure")
}

pub fn custom_keep_boot(bootloader: &str) -> String {
    match bootloader {
        "systemd" => "systemd-boot does not use a separate /boot".to_string(),
        _ => format!("{bootloader} installs with a supported ext4 /boot"),
    }
}

/// The installer asks this once, after the form is complete and before it
/// writes to the disk. The question carries the whole cost of installing.
pub fn erasing(disk: &str) -> String {
    format!("Erase everything on {disk}? This cannot be undone.")
}

pub fn changing_partitions(disk: &str) -> String {
    format!("Erase the partitions marked format on {disk}? This cannot be undone.")
}

/// The layout removes partitions the user never marked for format, and one
/// of them may carry another operating system. `changing_partitions` would
/// tell the user the wrong thing, so this question says DELETE instead. The
/// summary lists each removed partition by name below it.
pub fn removing_partitions(disk: &str, count: usize) -> String {
    match count {
        1 => format!("DELETE 1 partition on {disk}? Its contents are lost."),
        count => format!("DELETE {count} partitions on {disk}? Their contents are lost."),
    }
}

/// `DELETED` is deliberately the loudest word on the summary screen.
pub fn summary_deleted(partition: &str) -> String {
    format!("{partition}  DELETED, contents lost")
}

/// The confirmation names each ESP boot entry the install removes, because
/// the system it belonged to is rewritten or deleted.
pub fn summary_esp(entry: &str, partition: &str) -> String {
    format!("{entry} boot entry on {partition} is removed")
}
pub fn summary_created(gb: u64, fstype: &str) -> String {
    format!("a new {gb} GB partition, format as {fstype}")
}
pub const ROW_REMOVED: &str = "removed";
pub const ROW_CREATED: &str = "created";

pub const INSTALLATION_SUMMARY: &str = "Installation Summary";
pub const READY: &str = "Ready to install?";
pub const START_INSTALLATION: &str = "Start Installation";

/// The form's legend omits Esc, because the escape hatch already has a
/// button of its own in `EXIT_SHELL`.
pub const INSTALL_KEYS: &str = "\u{2191}\u{2193} navigate \u{2022} \u{23ce}  select";

pub const INSTALL: &str = "Install";
pub const SHUT_DOWN: &str = "Shut down";
pub const GO_BACK: &str = "Go back";
pub const EXIT_SHELL: &str = "Switch to shell";

// `Switch to shell` hands the console over. The user needs one fact before
// that happens, which is how to come back.

/// The note names the installer's own tty and the key that returns to it.
/// The calling command reads the VT number from the machine, because the
/// installer cannot assume which VT it runs on.
pub fn switch_note(vt: u32) -> String {
    format!("Installer on tty{vt}; press Ctrl+Alt+F{vt} to return.")
}
pub const ROW_CONFIRM: &str = "password (confirm)";
pub const ROW_LAYOUT: &str = "partition layout";
/// If the user answers `whole disk`, then the installer cuts the disk
/// itself. The partition table below the row carries the detail of a manual
/// answer.
pub const LAYOUT_WHOLE: &str = "whole disk";
pub const LAYOUT_MANUAL: &str = "manual";

/// These five head the table's value columns. `ROW_DISK` heads the label
/// column to their left, which carries the partition names.
pub fn layout_headings() -> [&'static str; 5] {
    ["size", "filesystem", "format", "type", "mount"]
}

/// Gives the table's value columns their fixed widths, one per heading. The
/// widest thing each column can hold sets its number: `100.1 GB` and the
/// automatic plan's `the rest` the size, `luks(closed)` the filesystem, the
/// heading alone carries the format, the longest type word (`linux`) the type,
/// and `/boot/efi` the mount.
pub fn layout_widths() -> [usize; 5] {
    [8, 12, 6, 5, 9]
}
pub const ASSIGN: &str = "Assign";
pub const UNASSIGN: &str = "Unassign";
pub const FORMAT_ROW: &str = "Format";
/// The `Format` overlay's legend says `confirm` where the table menu says
/// `select`, because one key takes the overlay and it opens no child menu.
pub const SELECT_FORMAT: &str = "select partition format";
pub const USE_DISK: &str = "use this disk";
pub const USE_DISK_ACTION: &str = "Use this disk";
pub const CHOOSE_KEYS: &str =
    "\u{2191}\u{2193} navigate \u{2022} \u{23ce}  confirm \u{2022} Esc back";
pub const RESET_CHANGES: &str = "Reset changes";
/// These four actions change the partition table itself. `cut_partitions`
/// enacts them through `sfdisk` before the installer formats any partition.
pub const DELETE_PART: &str = "Delete";
pub const CLEAR_PARTS: &str = "Clear partitions";
pub const CREATE_PART: &str = "Create partition";
pub const RENAME_PART: &str = "Rename";
pub const NEW_PARTITION: &str = "create partition";
pub const NEW_NAME: &str = "name";
pub const NAME_NEEDED: &str = "a name is needed";
/// The create window asks where the partition starts and how much of the free
/// region it takes. The start offset is measured from the region's first
/// sector.
pub const ROW_OFFSET: &str = "Start offset";
pub const ROW_PARTITION_SIZE: &str = "Partition size";
pub const ROW_AVAILABLE: &str = "Available";
pub const NEW_SIZE_NEEDED: &str = "a size is needed";
/// The `format` column says what will be written to a partition. These two
/// cells are the strongest answers that column takes.
pub const CELL_REMOVE: &str = "remove";
pub const CELL_NEW: &str = "new";
/// This action plans away partitions the user never looked at, so the
/// confirmation says how many.
pub fn clear_all(count: usize) -> String {
    format!("plan to remove all {count} partitions on this disk?")
}
pub const OPEN_ENCRYPTED: &str = "Open Encrypted Partition";
pub const CLOSE_ENCRYPTED: &str = "Close Encrypted Partition";
pub const NO_FORMAT: &str = "Do not format";
pub const FORMAT_TICK: &str = "\u{2713}";
/// The filesystem column says `luks` and whether the container is open.
/// `lsblk` reports `crypto_LUKS`, which the user has no reason to read. The
/// space is dropped so `luks(closed)` fits the column's twelve cells.
pub const LUKS_OPEN: &str = "luks(open)";
pub const LUKS_CLOSED: &str = "luks(closed)";
pub const EFI_CELL: &str = "efi";

/// The Assign list offers a partition these mount points. `assigns` leaves
/// `/boot` out where the target's bootloader cannot read a separate one.
pub fn mount_points() -> [&'static str; 4] {
    ["/boot/efi", "/", "/boot", "/swap"]
}

/// The `select partition format` overlay offers these, in the mock's order.
/// The mock's list carries no `ext3`, no `swap` and no `Do not format`.
/// `luks` makes the partition a container instead of a plain filesystem.
pub fn format_options() -> [&'static str; 5] {
    ["btrfs", "ext4", "fat32", "xfs", "luks"]
}

/// Offers a created partition every `format_options` entry but `luks`.
/// `collect` refuses a manual container through `CUSTOM_LUKS_LATER`, which
/// reads the layout's mounts and never a create, so `luks` on a create would
/// be a dead end the screen never refuses.
pub fn plain_formats() -> [&'static str; 4] {
    ["btrfs", "ext4", "fat32", "xfs"]
}

pub fn boot_chain(boot: &str) -> Option<&'static str> {
    match boot {
        "uki-shim" => Some("owner-signed UKI through Microsoft shim"),
        "uki-db" => Some("owner-signed UKI through firmware keys"),
        _ => None,
    }
}

/// Says a size the way the screens say it, with one decimal place in GB, so
/// the size column reads as one column. `lsblk` counts in binary units and the
/// screens say decimal GB, so a size from `lsblk` is converted rather than
/// relabelled. A size already in GB is reprinted to one decimal place. A value
/// that is not a number at all is returned unchanged.
pub fn size_said(size: &str) -> String {
    let number: String = size
        .chars()
        .take_while(|letter| letter.is_ascii_digit() || *letter == '.')
        .collect();
    let unit: String = size.chars().skip(number.len()).collect();
    let Ok(number) = number.parse::<f64>() else {
        return size.to_string();
    };
    let bytes = match unit.as_str() {
        "K" => number * 1024.0,
        "M" => number * 1024.0 * 1024.0,
        "G" => number * 1024.0 * 1024.0 * 1024.0,
        "T" => number * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "" | " GB" => return format!("{number:.1} GB"),
        _ => return size.to_string(),
    };
    format!("{:.1} GB", bytes / 1_000_000_000.0)
}

/// Refuses a created root smaller than the room the image needs for the
/// installed copy and the staged deployment beside it. The refusal belongs
/// here because `bootc` would discover it only after the disk had been cut
/// and formatted, with the old partition table already gone.
pub fn custom_root_too_small(gb: u64, reserve: u64) -> String {
    format!("a {gb} GB root is below the {reserve} GB this image needs")
}

/// The confirmation spells out one row per partition the install will cut.
/// No question on the form above covers these rows.
pub fn written_over(bootloader: &str, filesystem: &str) -> Vec<(String, String)> {
    // The rows follow the order the plan cuts in. The signed boot chain
    // belongs to the image and is drawn in the panel, so it is not a row
    // here.
    let esp = crate::ESP_GB.to_string();
    let mut rows = vec![("esp".to_string(), format!("{}  fat32", size_said(&esp)))];
    if bootloader != "systemd" {
        let boot = crate::BOOT_GB.to_string();
        rows.push(("/boot".to_string(), format!("{}  ext4", size_said(&boot))));
    }
    rows.push(("root".to_string(), format!("the rest  {filesystem}")));
    rows
}
pub const ROW_DISK: &str = "disk";
/// The form draws this as the table field's own label, above its columns.
pub const DISK_SELECTION: &str = "disk selection";
pub const ROW_HOSTNAME: &str = "hostname";
pub const ROW_ACCOUNT: &str = "username";
pub const ROW_PASSWORD: &str = "password";
pub const ROW_ENCRYPTION: &str = "encryption";
pub const ROW_PASSPHRASE: &str = "passphrase";
pub const ROW_PIN: &str = "PIN";
pub const PASSWORD_SET: &str = "set";
/// A form has to show the difference between a value and a gap, so an
/// unanswered field reads as this rather than as blank.
pub const NOT_SET: &str = "not set";

/// The dim action says which fields are still missing. Nothing derives these
/// fields and no default stands in for them.
pub fn still_needs(fields: &[&str]) -> String {
    format!("Missing: {}", fields.join(", "))
}

/// The title bar names the image and the current screen, so a user looking
/// at a photograph of the screen knows both.
pub fn installing(image: &str) -> String {
    format!("{image} installer \u{2022} configure")
}

/// A live environment may have nothing writable on it. The installer says so
/// outright, because the user cannot tell a log that was never written from
/// one they cannot find.
pub fn logging(log: Option<&std::path::Path>) -> String {
    match log {
        Some(at) => format!("the install log is at {}", at.display()),
        None => "nothing here is writable, so this screen is the only copy".to_string(),
    }
}

/// A user watching an install cannot ask for two facts. This line says that
/// the disk is being overwritten and where the transcript is. It stays short,
/// because it draws as one line inside a box.
pub fn writing(log: Option<&std::path::Path>) -> String {
    match log {
        Some(at) => format!("disk being overwritten; log: {}", at.display()),
        None => "disk being overwritten; this screen is the only log".to_string(),
    }
}

/// This key is the only copy the user will ever get. The disk does not open
/// without it if the TPM stops answering.
pub fn recovery(key: &str) -> String {
    format!("{} {key}", write_down())
}

/// The completion screen draws this above the key. Where there is no screen
/// to draw on, `recovery` appends the key to this line instead.
pub fn write_down() -> &'static str {
    "Write it down; it is the only copy."
}

pub const RECOVERY_HEADING: &str = "LUKS disk encryption recovery key:";
pub const NEXT_STEPS_HEADING: &str = "Next Steps:";

pub const LEAVING: &str = "Leave the installer?";
pub const LEAVE_BACK: &str = "Keep going";
pub const LEAVE_OVER: &str = "Start the form again";
/// On installer media the unit starts the installer again, so quitting
/// returns the user to a shell only where they ran the installer from one.
pub const LEAVE_SHELL: &str = "Quit the installer";
pub const INSTALL_DONE: &str = "Installation Complete";
pub const RESTART: &str = "Restart now";
/// The legend already names esc, so the last screen draws no row for it.
pub const DONE_KEYS: &str = "enter to finalize, esc to quit the installer";
/// An install whose first boot would ask for a key offers two ways to finish.
/// The automatic action writes a one-time key that the firmware carries into
/// the initrd, and the first boot removes it after the TPM is enrolled.
pub const FINALIZE_AUTO: &str = "Restart and finalize (automatic)";
pub const FINALIZE_MANUAL: &str = "Restart and finalize (manual)";
/// The completion screen explains what the automatic action does. Each
/// paragraph wraps to its own run of rows, as the firmware steps do.
pub fn automatic_explanation() -> [&'static str; 2] {
    [
        "This reboots once to enrol LUKS with the TPM. A temporary TPM-encrypted \
         key is staged on the ESP and unlocks the disk on the next boot; the first \
         boot then enrols the TPM, removes the temporary key entry from LUKS and \
         deletes the temporary key before restarting.",
        "Choose 'Manual' below to unlock the next boot with the recovery key \
         instead.",
    ]
}
/// The automatic action's window. The credential opens the volume until the
/// first boot kills the one-time slot, and a PIN kind loses its PIN for that
/// window alone.
pub const AUTO_WINDOW: &str = "until then this key opens the disk without a passphrase";
pub const AUTO_WINDOW_PIN: &str = "until then this key opens the disk without the PIN";
/// Why the automatic action draws dim and unpickable.
pub const AUTO_NO_STUB: &str = "the image does not boot through systemd-stub";
pub const AUTO_NO_POLICY: &str = "the image has no signed PCR policy to seal to";
pub const AUTO_NO_ROOT: &str = "no encrypted root was found";
/// The automatic action failed after the key was added. The completion screen
/// draws again with this reason, so the recovery key stays on it.
pub const AUTO_FAILED: &str = "the automatic finalize failed; choose manual restart";

/// The last screen asks for setup mode where a `uki-db` image's platform key
/// is still not the owner's. Vendors name that mode differently, so the text
/// gives the common wording too.
pub fn next_steps_setup() -> &'static str {
    "Before the installed image boots, set secure boot to 'setup mode'. The steps are vendor specific: often set secure boot to 'custom', then delete the existing platform key (PK)."
}

/// The shim chain asks the user for one confirmation and no firmware change.
/// The panel says it before the install and the last screen repeats it
/// after.
pub fn shim_steps() -> &'static str {
    "On first boot, use MokManager to enrol EFI/BOOT/MOK.cer; no firmware change is needed."
}

/// The panel draws each section title as one short line. No row under them
/// asks the user to read a certificate subject.
pub const PANEL_IMAGE: &str = "OS Image";
pub const PANEL_FIRMWARE: &str = "UEFI Firmware";
pub const PANEL_REQUIRED: &str = "Required";
/// The form asks its sections in this order.
pub const SETUP_OS: &str = "OS setup";
pub const SETUP_DISK: &str = "Disk setup";
pub const FIRMWARE_NO_EFIVARS: &str = "no EFI variables; not a UEFI boot";
/// The firmware panel tints only the secure-boot answer, so the label keeps
/// the panel's own ink.
pub const FIRMWARE_SECURE: &str = "secure boot";
pub const FIRMWARE_ON: &str = "on";
pub const FIRMWARE_OFF: &str = "off";
/// Setup mode is off with a difference. The installer can enrol keys in it
/// without sending the user into the firmware's own menus.
pub const FIRMWARE_SETUP: &str = "off (setup mode)";
pub const FIRMWARE_UNREADABLE: &str = "state unreadable";
/// The panel draws this label in the form's label column, so each answer
/// below carries the value alone.
pub const PLATFORM_KEY: &str = "platform key";
pub const PLATFORM_KEY_SETUP: &str = "none (setup mode)";
pub const PLATFORM_KEY_PRESENT: &str = "existing";
pub const PLATFORM_KEY_OWNER: &str = "the owner key";
pub const PLATFORM_KEY_UNKNOWN: &str = "could not be read";
pub const UKI_NEEDS_UEFI: &str = "this image needs a UEFI boot, and none was found";
pub const UKI_DB_ENROLL: &str = "the image enrolls its owner key on first boot";
pub const UKI_DB_OWNER: &str = "the owner key is enrolled; nothing to change";

/// The Required rows warn the user of a `uki-db` image that their platform
/// key will be deleted. The vendor's own steps come after the install, so no
/// row carries them. The panel wraps each row to its own width and draws the
/// empty row as a blank line.
pub fn required_uki_db() -> [&'static str; 3] {
    [
        "Secure boot with the systemd-boot bootloader needs the image's platform key set in this system's UEFI.",
        "",
        "The existing platform key is deleted, so any existing OS here stops booting with secure boot on.",
    ]
}

/// The last screen before the disk is written repeats the platform-key fact
/// in the warning colour. Two short rows replace `required_uki_db`'s prose,
/// because a `pub const` here has to stay under 60 characters.
pub const ERASE_WARNING_SECURE_BOOT: &str = "secure boot: the platform key must be cleared";
pub const ERASE_WARNING_UEFI_SETUP: &str = "afterwards, in UEFI setup: Custom mode";

/// An unanswered row reads as this word. `ENC_NONE` says the same word for
/// the encryption row, and the two answers are not the same answer.
pub const NONE: &str = "none";

#[cfg(test)]
mod tests {
    /// The question that costs a disk names the disk and asks. It is the last
    /// thing between the user and a wipe.
    #[test]
    fn the_cost_names_the_disk_and_asks() {
        let said = super::erasing("/dev/vda");
        assert!(said.contains("/dev/vda"), "{said}");
        assert!(said.contains('?'), "{said}");
    }

    /// A widget draws a question into a one-line head, so a question that
    /// wraps loses everything after its first line. This test reads the file
    /// itself, so a question added later is checked without a case being
    /// added here for it.
    #[test]
    fn every_string_here_is_one_short_line() {
        for line in include_str!("copy.rs").lines() {
            let Some(rest) = line.strip_prefix("pub const ") else {
                continue;
            };
            // A constant split over two lines does not end its first line in
            // a string, so this test skips it.
            if !rest.ends_with("\";") {
                continue;
            }
            let name = rest.split_once(':').expect("a const name").0;
            // A legend draws along the widget's border, so it may run past
            // the short-line limit a panel row has to keep.
            if name.ends_with("_KEYS") {
                continue;
            }
            let text = rest.split_once("= \"").expect("a string literal").1;
            assert!(text.chars().count() - 2 < 60, "{line}");
        }
    }

    #[test]
    fn uki_chains_are_visible_before_and_after_install() {
        assert!(super::boot_chain("uki-shim").is_some_and(|chain| chain.contains("Microsoft shim")));
        assert!(!super::written_over("systemd", "ext4")
            .iter()
            .any(|(name, _)| name == "boot chain"));
        assert!(super::next_steps_setup().contains("setup mode"));
        assert!(
            super::next_steps_setup().contains("platform key"),
            "the step that clears a foreign key is named"
        );
        assert!(super::shim_steps().contains("EFI/BOOT/MOK.cer"));
    }

    /// The enrolment names both files the stub places in `/run/systemd/` and
    /// keeps PCR 7. A silent bind to no PCRs at all is the failure this test
    /// guards against.
    #[test]
    fn a_pcr_policy_tpm2_unit_names_the_embedded_files() {
        let policy = super::tpm2_unit("root", "/etc/tect/tpm2-enroll-root.key", None, "u", true);
        assert!(policy.contains("--tpm2-pcrs=7"), "{policy}");
        assert!(
            policy.contains("--tpm2-public-key=/run/systemd/tpm2-pcr-public-key.pem"),
            "{policy}"
        );
        assert!(
            policy.contains("--tpm2-signature=/run/systemd/tpm2-pcr-signature.json"),
            "{policy}"
        );
        let other = super::tpm2_unit("root", "/etc/tect/tpm2-enroll-root.key", None, "u", false);
        assert!(other.contains("--tpm2-pcrs=7"), "{other}");
        assert!(!other.contains("tpm2-pcr-public-key"), "{other}");
        assert!(!other.contains("tpm2-with-pin"), "{other}");
        assert!(!other.contains("new-tpm2-pin"), "{other}");
    }

    /// Without the ordering, the unit races `getty@tty1` and prints over a
    /// login prompt the user may already be typing into.
    #[test]
    fn a_tpm2_unit_runs_before_any_login_and_says_so() {
        let unit = super::tpm2_unit("root", "/etc/tect/tpm2-enroll-root.key", None, "u", false);
        assert!(
            unit.contains("Before=systemd-user-sessions.service"),
            "{unit}"
        );
        assert!(unit.contains("TimeoutStartSec=120"), "{unit}");
        assert!(unit.contains("StandardOutput=journal+console"), "{unit}");
        assert!(unit.contains("StandardError=journal+console"), "{unit}");
        assert!(
            unit.contains(&format!("ExecStartPre=/bin/echo '{}'", super::ENROLLING)),
            "{unit}"
        );
        assert!(
            super::ENROLLING.chars().count() < 60,
            "{}",
            super::ENROLLING
        );
    }

    /// The automatic finalize leaves a marker on the ESP, and the unit that
    /// enrols the token must also kill the one-time slot and remove the
    /// credential. A failed cleanup keeps the key, so the next boot runs the
    /// unit again.
    #[test]
    fn a_tpm2_unit_cleans_up_an_automatic_finalize() {
        let unit = super::tpm2_unit("root", "/etc/tect/tpm2-enroll-root.key", None, "abcd", true);
        assert!(unit.contains(&format!("\"{}\"", super::ESP_TYPE)), "{unit}");
        assert!(
            unit.contains("loader/credentials/cryptsetup.slot"),
            "{unit}"
        );
        assert!(
            unit.contains("loader/credentials/cryptsetup.passphrase.cred"),
            "{unit}"
        );
        assert!(unit.contains("--wipe-slot=$slot"), "{unit}");
        assert!(
            unit.contains("--unlock-key-file=/etc/tect/tpm2-enroll-root.key $wipe"),
            "{unit}"
        );
        assert!(unit.contains("for i in 1 2 3 4 5"), "{unit}");
        assert!(
            unit.contains("rm -f \"$mnt/loader/credentials/cryptsetup.passphrase.cred\"")
                && unit.contains("|| exit 1"),
            "{unit}"
        );
    }

    #[test]
    fn a_pin_unit_loads_and_shreds_the_pin() {
        let unit = super::tpm2_unit(
            "root",
            "/etc/tect/tpm2-enroll-root.key",
            Some("/etc/tect/tpm2-enroll-root.pin"),
            "abcd",
            false,
        );
        assert!(unit.contains("--tpm2-with-pin=yes"), "{unit}");
        assert!(
            unit.contains("LoadCredential=cryptenroll.new-tpm2-pin:/etc/tect/tpm2-enroll-root.pin"),
            "{unit}"
        );
        assert!(
            unit.contains("ExecStartPost=-/usr/bin/shred -u /etc/tect/tpm2-enroll-root.pin"),
            "{unit}"
        );
    }
}
