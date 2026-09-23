use super::*;

impl Answers {
    /// Asks the install screens in order, then asks the review over their
    /// answers. Every leave key asks its own question first. `None` means the
    /// user confirmed a leave, so the installer has written nothing and has
    /// touched no disk.
    pub fn collect(
        payload: &Payload,
        given: Given,
        prompt: &Prompt,
    ) -> Result<Option<Self>, String> {
        let named = given.encryption.is_some();
        let seeded = Self::seeded(payload, given, prompt)?;
        // A headless run draws no form, so the flags are the whole answer.
        // `seeded` has already refused a flag the user left empty.
        if !prompt.draws() {
            return Ok(Some(seeded));
        }
        let found_disks = disks(&sys_block(), &in_use_now());
        let mut disk = seeded.disk.clone();
        // One found disk makes the disk question answer itself. The install
        // opens with that disk taken.
        if disk.is_empty() && found_disks.len() == 1 {
            disk = found_disks[0].0.clone();
        }
        // The whole-disk kind lives outside the form field. A manual layout
        // that opens a container replaces the encryption row, and a return
        // through the whole-disk answer must keep the kind the user chose. An
        // unanswered form installs encrypted where the TPM and the image both
        // allow it. A flag naming a kind keeps that kind.
        let chosen = seeded.encryption.kind.clone();
        let mut kind = match (
            named,
            chosen.as_str(),
            tpm().exists() && payload.luks_initramfs,
        ) {
            (false, NONE, true) => "tpm2-luks".to_string(),
            _ => chosen,
        };
        let mut layout: Option<CustomLayout> = None;
        let mut fields = seeded.fields(payload, &disk, layout.as_ref(), "", &[]);
        let panel = panel(payload);
        // The disk table keeps its own cursor across the form reopens, so the
        // walk stays on the row the user left. `None` seeds it on the chosen
        // disk.
        let mut cursor: Option<usize> = None;
        // Reopens the form on the disk table after the next pass rebuilds the
        // rows around it. Opening a container makes the encryption row a
        // question the manual form otherwise hides, which moves the table's
        // row number.
        let mut reopen_on_table = false;
        // Holds the keys the walk found on old systems, read once per disk.
        let mut found = Discovered::default();
        let mut found_disk = String::new();
        // Names the row the form opens on. The disk table returns to its own
        // row, because the top of the form is a long walk back through the
        // setup rows above it.
        let mut start = 0;
        // Records which password rows the user has typed in and left, carried
        // across the form's reopens. An answer on the disk table or the layout
        // row rebuilds `fields` and calls `form` again. A mismatched password
        // pair keeps its red through that redraw.
        let mut left: Vec<usize> = Vec::new();
        loop {
            // The layout row decides what the layout holds. A manual layout
            // always holds one for the chosen disk, and a whole-disk layout
            // holds none. A layout built for another disk is dropped here.
            let manual = fields[ROW_LAYOUT].value() == copy::LAYOUT_MANUAL;
            layout = match (manual, layout.take()) {
                (true, Some(held)) if held.disk == disk => Some(held),
                (true, _) => Some(CustomLayout::empty(&disk)),
                (false, _) => None,
            };
            // A manual layout draws the table from every found disk's
            // partitions. A whole-disk layout reads none of them.
            let partitions: Vec<(String, Vec<Partition>)> = match manual {
                true => found_disks
                    .iter()
                    .map(|(at, _)| Ok((at.clone(), partitions(at)?)))
                    .collect::<Result<_, String>>()?,
                false => Vec::new(),
            };
            let chosen = partitions
                .iter()
                .find(|(at, _)| at == &disk)
                .map(|(_, parts)| parts.as_slice())
                .unwrap_or(&[]);
            // Reads the disk's partition table on every redraw, so `short_of`
            // re-takes the room refusal after any answer and not only after a
            // size is typed. Taking back a delete after a create was sized
            // would otherwise leave a plan the disk cannot hold. `sfdisk`
            // would then find it while running, after the other deletes were
            // written.
            let table_now = match manual && !disk.is_empty() {
                true => Some(disk_table(&disk)?),
                false => None,
            };
            if manual && found_disk != disk {
                found = old_keys(&disk, chosen);
                found_disk = disk.clone();
            }
            if !manual {
                found = Discovered::default();
                found_disk.clear();
            }
            // A home size the user typed stops shaping the plan once the home
            // row goes back to sharing the root. The home row gates the size.
            let separate = fields[ROW_DATA].value() == copy::DATA_SEPARATE;
            let size = match separate {
                true => fields[ROW_SIZE].value(),
                false => String::new(),
            };
            let table = layout_table(
                &found_disks,
                &disk,
                layout.as_ref(),
                &partitions,
                payload,
                &size,
                // The plan draws a separate home partition before its size is
                // typed.
                separate,
                // The whole-disk kind answers this, because a layout with open
                // containers replaces the encryption row.
                kind != NONE,
            );
            let at = match cursor {
                Some(at) => at,
                None => table
                    .kinds
                    .iter()
                    .position(|kind| matches!(kind, RowKind::Disk(at) if at == &disk))
                    .unwrap_or(0),
            };
            fields[ROW_TABLE] = table.field(clamp_row(&table.selectable, at), !disk.is_empty());
            // The encryption row asks what the container headers hold once a
            // manual layout opens one. Otherwise it shows the whole-disk
            // kinds.
            fields[ROW_ENCRYPTION] = match layout.as_ref().filter(|layout| !layout.opens.is_empty())
            {
                Some(layout) => encryption_row(
                    Some(layout),
                    &fields[ROW_ENCRYPTION],
                    tpm().exists(),
                    payload.luks_initramfs,
                ),
                // A manual layout with no open container still shows the
                // whole-disk kinds. `asked` hides that row, `Opened` ignores
                // it, and `of` drops its value, so the layout never carries
                // the whole-disk answer.
                None if manual => common::ui::Field::action(copy::ROW_ENCRYPTION, shown(&kind)),
                None => common::ui::Field::action(copy::ROW_ENCRYPTION, shown(&kind)),
            };
            // The disk table was just answered and the rows around it are
            // rebuilt, so the form opens on the table's new row number.
            if reopen_on_table {
                reopen_on_table = false;
                start = ROW_TABLE;
            }
            let actions = [
                copy::INSTALL,
                copy::EXIT_SHELL,
                copy::RESTART,
                copy::SHUT_DOWN,
            ];
            let filled = common::ui::form(
                &mut fields,
                &actions,
                |fields| {
                    short_of(
                        fields,
                        &disk,
                        layout.as_ref(),
                        payload,
                        tpm().exists(),
                        table_now.as_ref(),
                    )
                },
                |fields| pair_mismatch(fields, [ROW_PASSWORD, ROW_CONFIRM]),
                |fields| asked(fields),
                copy::INSTALL_KEYS,
                &panel,
                &[
                    (ROW_HOSTNAME, copy::SETUP_OS),
                    (ROW_LAYOUT, copy::SETUP_DISK),
                ],
                start,
                &mut left,
            );
            match filled {
                Ok(common::ui::Filled::Took(0)) => {
                    let mut answers = Self::of(&fields, disk.clone(), layout.clone());
                    if let Some(layout) = answers
                        .layout
                        .as_mut()
                        .filter(|layout| !layout.deletes.is_empty() || !layout.creates.is_empty())
                    {
                        let shown = table_now
                            .as_ref()
                            .ok_or("the partition table was not available to confirm")?;
                        let state = disk_state(&layout.disk)?;
                        if state.table != *shown {
                            return Err(copy::table_changed(&layout.disk));
                        }
                        layout.confirmed = Some(state);
                    }
                    let warning = erase_warning(&payload.boot, &firmware(payload));
                    // Asks the one question that costs the user a disk. The
                    // installer asks it over the plan, and only once the form
                    // is complete.
                    if common::ui::decide(
                        copy::INSTALLATION_SUMMARY,
                        &match &answers.layout {
                            // A layout that removes partitions is asked over
                            // the removal, which costs the user more than a
                            // format does.
                            Some(layout) if !layout.deletes.is_empty() => {
                                copy::removing_partitions(&answers.disk, layout.deletes.len())
                            }
                            Some(_) => copy::changing_partitions(&answers.disk),
                            None => copy::erasing(&answers.disk),
                        },
                        &warning,
                        &answers.summary(payload),
                        copy::READY,
                        copy::START_INSTALLATION,
                        copy::GO_BACK,
                    )? {
                        return Ok(Some(answers));
                    }
                }
                // Leaves the install for a root shell, which is what the live
                // environment is for. Under the media's installer unit the
                // shell takes another VT and the user can come back. Started
                // by hand, the installer exits, so the user confirms first.
                Ok(common::ui::Filled::Took(1)) => {
                    // The shell takes another VT, so the installer keeps its
                    // screen. The note names the key that comes back to it.
                    let note = copy::switch_note(console_vt().unwrap_or(1));
                    if common::ui::decide(
                        copy::EXIT_SHELL,
                        &note,
                        &[],
                        &[],
                        copy::EXIT_SHELL,
                        copy::EXIT_SHELL,
                        copy::GO_BACK,
                    )? {
                        // An installer the user started by hand has no console
                        // of its own, so it returns to the shell that ran it.
                        if !under_media_installer() {
                            return Ok(None);
                        }
                        switch_to_shell()?;
                    }
                }
                // Nothing is written yet, so the restart costs the user no disk.
                Ok(common::ui::Filled::Took(2)) => {
                    restart()?;
                    return Ok(None);
                }
                // Shut down is the last of the four actions, so it takes this arm.
                Ok(common::ui::Filled::Took(_)) => {
                    power_off()?;
                    return Ok(None);
                }
                // The user answered the disk table. A disk row picks the disk,
                // and a partition row answers through the popup the table
                // opened for it.
                Ok(common::ui::Filled::Table { row, item, child }) => {
                    cursor = Some(row);
                    reopen_on_table = true;
                    match table.kinds.get(row) {
                        Some(RowKind::Disk(at)) => {
                            let action = item
                                .and_then(|at| table.actions.get(row).and_then(|row| row.get(at)))
                                .copied();
                            match action {
                                Some(PartAction::Create) => {
                                    // Offers the room after the last surviving
                                    // partition, less what the plan already
                                    // spends. The size question is about the
                                    // disk as it stands now.
                                    let planned: u64 = layout
                                        .as_ref()
                                        .map(|held| {
                                            held.creates.iter().map(|create| create.gb).sum()
                                        })
                                        .unwrap_or_default();
                                    let deletes = layout
                                        .as_ref()
                                        .map(|held| held.deletes.clone())
                                        .unwrap_or_default();
                                    let room = disk_table(at)?
                                        .appendable_gb(&deletes)
                                        .saturating_sub(planned);
                                    if let Some(gb) = ask_size(room)? {
                                        held_layout(&mut layout, &disk).creates.push(Created {
                                            gb,
                                            ..Default::default()
                                        });
                                    }
                                }
                                // Clear answers for partitions the user never
                                // opened, so the question names how many.
                                Some(PartAction::Clear) => {
                                    if common::ui::confirm(
                                        &copy::clear_all(chosen.len()),
                                        copy::USE_DISK_ACTION,
                                        copy::GO_BACK,
                                    )? {
                                        let held = held_layout(&mut layout, &disk);
                                        held.deletes =
                                            chosen.iter().map(|part| part.device.clone()).collect();
                                        // A partition planned away keeps no
                                        // mount and no open.
                                        held.mounts.clear();
                                        held.opens.clear();
                                    }
                                }
                                // A whole-disk pick erases the disk, so the
                                // user is shown what is on it first. A node
                                // alone is not enough to recognise a drive by.
                                // A manual layout already carries the disk's
                                // partitions on the form.
                                _ => {
                                    if manual || confirm_disk(at, &found_disks)? {
                                        disk = at.clone();
                                    }
                                }
                            }
                        }
                        // A planned partition is on no disk yet, so its answers
                        // live on the plan entry. A node would move when
                        // another partition is removed.
                        Some(RowKind::Created { index }) => {
                            let index = *index;
                            let Some(action) = item
                                .and_then(|at| table.actions.get(row).and_then(|row| row.get(at)))
                                .copied()
                            else {
                                continue;
                            };
                            let children: &[String] = item
                                .and_then(|at| table.menus.get(row).and_then(|row| row.get(at)))
                                .map(|menu| menu.children.as_slice())
                                .unwrap_or(&[]);
                            match action {
                                PartAction::Assign => {
                                    if let Some(target) = child.and_then(|at| children.get(at)) {
                                        if let Some(create) = layout
                                            .as_mut()
                                            .and_then(|held| held.creates.get_mut(index))
                                        {
                                            create.target = match target.as_str() {
                                                copy::UNASSIGN => String::new(),
                                                target => target.to_string(),
                                            };
                                        }
                                    }
                                }
                                PartAction::Format => {
                                    // A created partition is offered the plain
                                    // formats. `customMounts` has no
                                    // passphrase field, so `short_of` refuses
                                    // a manual container with
                                    // `CUSTOM_LUKS_LATER`. That rule reads
                                    // `mounts` and a create is not one, so
                                    // offering `luks` here would be a dead end
                                    // with no refusal on screen.
                                    let options = copy::plain_formats();
                                    let current = layout
                                        .as_ref()
                                        .and_then(|held| held.creates.get(index))
                                        .map(|create| create.fstype.clone())
                                        .unwrap_or_default();
                                    let at = options
                                        .iter()
                                        .position(|option| *option == current.as_str())
                                        .unwrap_or(0);
                                    if let Some(taken) = common::ui::choose(
                                        copy::SELECT_FORMAT,
                                        copy::CHOOSE_KEYS,
                                        &options,
                                        at,
                                    )? {
                                        if let Some(create) = layout
                                            .as_mut()
                                            .and_then(|held| held.creates.get_mut(index))
                                        {
                                            create.fstype = options[taken].to_string();
                                            // The chosen filesystem overrules
                                            // the mount point, as it does on a
                                            // partition that was already
                                            // there. A point the new
                                            // filesystem cannot carry is
                                            // cleared instead of left wrong.
                                            if !create.target.is_empty()
                                                && !fits(&create.target, &create.fstype)
                                            {
                                                create.target.clear();
                                            }
                                        }
                                    }
                                }
                                // Dropping a create moves the creates under it
                                // up a slot. Their answers ride on the plan
                                // entry, so they move with it.
                                PartAction::Delete => {
                                    if let Some(held) = layout.as_mut() {
                                        if index < held.creates.len() {
                                            held.creates.remove(index);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        Some(RowKind::Part { index }) => {
                            let Some(partition) = chosen.get(*index) else {
                                continue;
                            };
                            let Some(action) = item
                                .and_then(|at| table.actions.get(row).and_then(|row| row.get(at)))
                                .copied()
                            else {
                                continue;
                            };
                            let children: &[String] = item
                                .and_then(|at| table.menus.get(row).and_then(|row| row.get(at)))
                                .map(|menu| menu.children.as_slice())
                                .unwrap_or(&[]);
                            match action {
                                PartAction::Assign => {
                                    if let Some(target) = child.and_then(|at| children.get(at)) {
                                        match target.as_str() {
                                            copy::UNASSIGN => {
                                                place_unassign(&mut layout, partition)
                                            }
                                            _ => {
                                                place_target(&mut layout, &disk, partition, target)
                                            }
                                        }
                                    }
                                }
                                PartAction::Format => {
                                    // The format overlay opens on the format the
                                    // partition row already holds.
                                    let options = copy::format_options();
                                    let answer =
                                        layout.as_ref().and_then(|held| held.answer(partition));
                                    let current = effective_fs(partition, answer.as_ref());
                                    let at = options
                                        .iter()
                                        .position(|option| *option == current.as_str())
                                        .unwrap_or(0);
                                    if let Some(taken) = common::ui::choose(
                                        copy::SELECT_FORMAT,
                                        copy::CHOOSE_KEYS,
                                        &options,
                                        at,
                                    )? {
                                        match options[taken] {
                                            // A `luks` partition is a container
                                            // before it is a filesystem, so
                                            // the installer asks what opens it
                                            // and then stages it.
                                            "luks" => {
                                                // Opens on `tpm2-luks-passphrase`,
                                                // the kind that unlocks from the
                                                // TPM and still keeps a passphrase
                                                // the user can be asked for.
                                                if let Some((_, passphrase, _)) = edit_luks(
                                                    KINDS[3].0,
                                                    "",
                                                    Some(&partition.device),
                                                    tpm().exists(),
                                                    payload.luks_initramfs,
                                                    // A new container owes a key,
                                                    // so `none` is left out.
                                                    false,
                                                )? {
                                                    place_format(
                                                        &mut layout,
                                                        &disk,
                                                        partition,
                                                        "luks",
                                                    );
                                                    if let Some(mount) =
                                                        layout.as_mut().and_then(|held| {
                                                            held.mounts.iter_mut().find(|mount| {
                                                                mount.partition == partition.device
                                                            })
                                                        })
                                                    {
                                                        // ponytail: the summary
                                                        // holds the key, because
                                                        // the recipe's
                                                        // `customMounts` cannot
                                                        // carry it yet. The
                                                        // fisherman field is
                                                        // phase 2 of plan 8G.
                                                        mount.passphrase = passphrase;
                                                    }
                                                }
                                            }
                                            format => {
                                                place_format(&mut layout, &disk, partition, format)
                                            }
                                        }
                                    }
                                }
                                PartAction::Reset => {
                                    if let Some(layout) = layout.as_mut() {
                                        layout
                                            .mounts
                                            .retain(|mount| mount.partition != partition.device);
                                        layout
                                            .opens
                                            .retain(|open| open.partition != partition.device);
                                        // Reset is the one item a partition
                                        // planned away still offers, so it is
                                        // also what puts it back.
                                        layout.deletes.retain(|at| at != &partition.device);
                                    }
                                }
                                // A partition planned away loses its answers.
                                // The install cannot mount, format or open a
                                // partition that will not be there.
                                PartAction::Delete => {
                                    let device = partition.device.clone();
                                    let held = held_layout(&mut layout, &disk);
                                    held.mounts.retain(|mount| mount.partition != device);
                                    held.opens.retain(|open| open.partition != device);
                                    if !held.deletes.contains(&device) {
                                        held.deletes.push(device);
                                    }
                                }
                                PartAction::Open => {
                                    let key = match open_key(
                                        &partition.device,
                                        layout.as_ref(),
                                        &found,
                                    ) {
                                        Ok(key) => key,
                                        // Esc on the key screen returns to the
                                        // disk table.
                                        Err(err) if leaving(&err) => continue,
                                        Err(err) => return Err(err),
                                    };
                                    let layout = held_layout(&mut layout, &disk);
                                    layout
                                        .mounts
                                        .retain(|mount| mount.partition != partition.device);
                                    layout
                                        .opens
                                        .retain(|open| open.partition != partition.device);
                                    layout.opens.push(LuksOpen {
                                        partition: partition.device.clone(),
                                        // The key is known before the mount point
                                        // is. Assign sets the target, and
                                        // `short_of` refuses an open with an
                                        // empty one.
                                        target: String::new(),
                                        key,
                                    });
                                }
                                PartAction::Close => {
                                    if let Some(layout) = layout.as_mut() {
                                        layout
                                            .opens
                                            .retain(|open| open.partition != partition.device);
                                    }
                                }
                                // The partition popup offers neither action.
                                PartAction::Clear | PartAction::Create => {}
                            }
                        }
                        _ => {}
                    }
                }
                // A changed pick row decides what the table under it draws.
                // The loop rebuilds the table and the rows before the form is
                // drawn again, and the form opens on the row the user answered.
                Ok(common::ui::Filled::Changed(row)) => {
                    start = row;
                }
                // The whole-disk encryption row opens a window. The user picks
                // the kind there and types the passphrase that kind owes. A
                // manual layout never reaches this arm.
                Ok(common::ui::Filled::Opened(ROW_ENCRYPTION)) if layout.is_none() => {
                    let at = (!disk.is_empty())
                        .then(|| fdisk::partname(&disk, container_number(payload)));
                    if let Some((chosen, passphrase, pin)) = edit_luks(
                        &kind,
                        &fields[ROW_PASSPHRASE].value(),
                        at.as_deref(),
                        tpm().exists(),
                        payload.luks_initramfs,
                        // The whole-disk row keeps `none`, because the root may
                        // stay plain.
                        true,
                    )? {
                        kind = chosen;
                        fields[ROW_PASSPHRASE] =
                            common::ui::Field::secret(copy::ROW_PASSPHRASE, &passphrase);
                        fields[ROW_PIN] = common::ui::Field::secret(copy::ROW_PIN, &pin);
                    }
                    // The next pass rebuilds the row from `kind`, so the form
                    // opens on it and the user sees the kind that was taken.
                    start = ROW_ENCRYPTION;
                }
                Ok(common::ui::Filled::Opened(_)) => {}
                Ok(common::ui::Filled::Left) => match leave(prompt)? {
                    Leave::Shell => return Ok(None),
                    Leave::Over => {
                        disk.clear();
                        kind = NONE.to_string();
                        layout = None;
                        cursor = None;
                        start = 0;
                        left.clear();
                        fields = Self::seeded(payload, Given::default(), prompt)?.fields(
                            payload,
                            &disk,
                            None,
                            "",
                            &[],
                        )
                    }
                    Leave::Back => {}
                },
                Err(err) if leaving(&err) => match leave(prompt)? {
                    Leave::Shell => return Ok(None),
                    Leave::Over => {
                        disk.clear();
                        kind = NONE.to_string();
                        layout = None;
                        cursor = None;
                        start = 0;
                        left.clear();
                        fields = Self::seeded(payload, Given::default(), prompt)?.fields(
                            payload,
                            &disk,
                            None,
                            "",
                            &[],
                        )
                    }
                    Leave::Back => {}
                },
                Err(err) => return Err(err),
            }
        }
    }

    /// Fills every answer before the installer asks the user a question. With
    /// a screen, the flags and the payload defaults seed the form and the form
    /// asks. With no screen, a value no flag gave is a refusal naming the
    /// flag.
    pub(crate) fn seeded(payload: &Payload, given: Given, prompt: &Prompt) -> Result<Self, String> {
        if !prompt.draws() {
            return Ok(Self {
                disk: ask_disk(given.disk, None, prompt)?,
                hostname: prompt.text(
                    given.hostname,
                    copy::INSTALL_NAME,
                    "--hostname",
                    Some(&payload.hostname),
                )?,
                user: prompt.text(given.user, copy::INSTALL_USER, "--user", None)?,
                password: prompt.text(
                    given.password,
                    copy::INSTALL_PASSWORD,
                    "--password",
                    None,
                )?,
                encryption: ask_encryption(
                    given.encryption,
                    given.passphrase,
                    given.pin,
                    prompt,
                    tpm().exists(),
                    payload.luks_initramfs,
                )?,
                // No flag names a separate home, so a headless run installs
                // without one.
                data: Data::default(),
                layout: None,
                opened: Opened::Keep,
            });
        }
        Ok(Self {
            disk: given.disk.unwrap_or_default(),
            hostname: given
                .hostname
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| payload.hostname.clone()),
            user: given.user.unwrap_or_default(),
            password: given.password.unwrap_or_default(),
            encryption: Encryption {
                // `named` refuses a flag naming a kind fisherman has not got.
                // Drawn as a row, the kind would be one the user cannot
                // correct.
                kind: match given.encryption {
                    Some(kind) => named(kind)?,
                    None => NONE.to_string(),
                },
                passphrase: given.passphrase.unwrap_or_default(),
                pin: given.pin.unwrap_or_default(),
                // `collect` chooses the interactive default, where the TPM and
                // the image answer whether they can take it.
            },
            data: Data::default(),
            layout: None,
            opened: Opened::Keep,
        })
    }

    /// Builds the form's rows in the order `ROW_*` names them. The passphrase
    /// and the PIN ride here for the encryption window, and `asked` keeps both
    /// out of the questions.
    pub(crate) fn fields(
        &self,
        payload: &Payload,
        disk: &str,
        layout: Option<&CustomLayout>,
        var_size: &str,
        partitions: &[(String, Vec<Partition>)],
    ) -> Vec<common::ui::Field> {
        use common::ui::Field;
        let found = disks(&sys_block(), &in_use_now());
        let table = layout_table(
            &found,
            disk,
            layout,
            partitions,
            payload,
            var_size,
            !self.data.size.is_empty(),
            self.encryption.kind != NONE,
        );
        vec![
            Field::text(copy::ROW_HOSTNAME, &self.hostname),
            Field::text(copy::ROW_ACCOUNT, &self.user),
            Field::secret(copy::ROW_PASSWORD, &self.password),
            Field::secret(copy::ROW_CONFIRM, &self.password),
            Field::pick_change(
                copy::ROW_LAYOUT,
                vec![
                    Choice::new(copy::LAYOUT_WHOLE, ""),
                    Choice::new(copy::LAYOUT_MANUAL, ""),
                ],
                Some(usize::from(layout.is_some())),
            ),
            encryption_row(
                layout,
                &Field::action(copy::ROW_ENCRYPTION, shown(&self.encryption.kind)),
                tpm().exists(),
                payload.luks_initramfs,
            ),
            // No flag names the home row, so it opens unanswered. Answering it
            // rebuilds the disk table, which draws the home partition this row
            // decides.
            Field::pick_change(copy::ROW_DATA, data_rows(), Some(0)),
            Field::measure(
                &format!("\u{2514}\u{2500} {}", copy::ROW_SIZE),
                &self.data.size,
                "GB",
            ),
            table.field(0, !disk.is_empty()),
            Field::secret(copy::ROW_PASSPHRASE, &self.encryption.passphrase),
            Field::secret(copy::ROW_PIN, &self.encryption.pin),
        ]
    }

    /// Reads the answers the form holds. `collect` calls it only once
    /// `short_of` is empty, so every value here is one the user typed or
    /// chose.
    pub(crate) fn of(
        fields: &[common::ui::Field],
        disk: String,
        layout: Option<CustomLayout>,
    ) -> Self {
        let at = |row: usize| fields[row].value();
        let held = held_pick(fields, layout.as_ref(), &disk);
        let manual = held.is_some();
        // A `Pick` on the encryption row means the manual layout has an open
        // container, and the row holds that container's kind.
        let opens = matches!(fields[ROW_ENCRYPTION], common::ui::Field::Pick { .. });
        Self {
            disk,
            hostname: at(ROW_HOSTNAME),
            user: at(ROW_ACCOUNT),
            password: at(ROW_PASSWORD),
            encryption: match manual {
                // A manual layout encrypts per partition, so the whole-disk
                // kinds are not its answer.
                true => Encryption {
                    kind: NONE.to_string(),
                    passphrase: String::new(),
                    pin: String::new(),
                },
                false => {
                    // The rows show descriptions and the recipe takes
                    // fisherman's names. A recipe naming a description is
                    // refused after the disk is gone.
                    let kind = written(&at(ROW_ENCRYPTION));
                    Encryption {
                        kind: kind.to_string(),
                        // A passphrase or PIN left from a kind the user turned
                        // off stays out of the recipe.
                        passphrase: match kind == NONE {
                            true => String::new(),
                            false => at(ROW_PASSPHRASE),
                        },
                        pin: match kind == "tpm2-luks-pin" {
                            true => at(ROW_PIN),
                            false => String::new(),
                        },
                    }
                }
            },
            data: match manual {
                true => Data::default(),
                false => chose(&at(ROW_DATA), &at(ROW_SIZE)),
            },
            opened: match manual {
                true if opens => Opened::of(&at(ROW_ENCRYPTION)),
                _ => Opened::Keep,
            },
            layout: held,
        }
    }

    /// Builds the rows the confirmation is asked over. The password is the one
    /// answer that cannot read back as itself.
    pub(crate) fn summary(&self, payload: &Payload) -> Vec<(String, String)> {
        let mut rows = vec![
            (copy::ROW_DISK.to_string(), self.disk.clone()),
            (copy::ROW_HOSTNAME.to_string(), self.hostname.clone()),
            (copy::ROW_ACCOUNT.to_string(), self.user.clone()),
            (
                copy::ROW_PASSWORD.to_string(),
                copy::PASSWORD_SET.to_string(),
            ),
            (
                copy::ROW_ENCRYPTION.to_string(),
                shown(&self.encryption.kind).to_string(),
            ),
        ];
        if self.layout.is_none() {
            rows.push((copy::ROW_DATA.to_string(), data_said(&self.data)));
        }
        // Names what the disk is about to be cut into. No question above
        // covers this half of what the install writes.
        match &self.layout {
            Some(layout) => {
                // Removals come first, because a partition that stops existing
                // costs the user most. `Clear partitions` empties `mounts` and
                // `opens`, so without these rows a layout that removes every
                // partition on the disk would show an empty summary under a
                // sentence about formatting.
                rows.extend(
                    layout.deletes.iter().map(|device| {
                        (copy::ROW_REMOVED.to_string(), copy::summary_deleted(device))
                    }),
                );
                // A created partition is named by its mount point, as a kept
                // or formatted one is. The disk table the user just left shows
                // the node it will get.
                rows.extend(layout.creates.iter().map(|create| {
                    (
                        create.target.clone(),
                        copy::summary_created(create.gb, &create.fstype),
                    )
                }));
                rows.extend(layout.mounts.iter().map(|mount| {
                    let how = match mount.fstype.as_str() {
                        "unformatted" => format!("{}  kept", mount.partition),
                        fstype => format!("{}  format as {fstype}", mount.partition),
                    };
                    (mount.target.clone(), how)
                }));
                rows.extend(layout.opens.iter().map(|open| {
                    (
                        open.target.clone(),
                        copy::opened(&open.partition, at_boot(open, self.opened)),
                    )
                }));
                if let Some(chain) = copy::boot_chain(&payload.boot) {
                    rows.push(("boot chain".to_string(), chain.to_string()));
                }
            }
            None => rows.extend(copy::written_over(
                &payload.bootloader,
                &payload.filesystem,
                &self.data.size,
                self.encryption.kind != NONE,
            )),
        }
        rows
    }
}
