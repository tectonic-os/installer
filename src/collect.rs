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
        // Every disk the machine has, read once before the form draws. The
        // screens render this and read no disk again, so a redraw cannot show
        // a different disk than the last one.
        let scan = Scan::read();
        let mut disk = seeded.disk.clone();
        // One found disk nothing is on yet makes the disk question answer
        // itself. A disk holding partitions is the user's to choose.
        if disk.is_empty() {
            disk = scan.only_empty_disk().unwrap_or_default();
        }
        // A disk whose partitions could not be read has no picture, so it is
        // not an install target. A machine with no readable disk left stops
        // the same way rather than drawing a form nothing can answer.
        if let Some(why) = scan.refusal(&disk) {
            return Err(why);
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
        let mut fields = seeded.fields(payload, &scan, &disk, layout.as_ref());
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
        // Opens the table with its own cursor once, for the pass after the user
        // answered a table row. The owner asked 2026-09-24 that an Assign,
        // Format or Delete leave the selection on the partition it was made on.
        let mut focus_table = false;
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
            // The chosen disk's partitions, from the scan. The table draws
            // every disk from the scan, so nothing is read here.
            let chosen = scan
                .get(&disk)
                .map(|entry| entry.partitions.as_slice())
                .unwrap_or(&[]);
            // `short_of` weighs the plan against the table the form drew, and
            // re-takes the room refusal after any answer rather than only
            // after a size is typed. Taking back a delete after a create was
            // sized would otherwise leave a plan the disk cannot hold. The
            // scan is what the user saw; `table_changed` at confirm remains
            // the guard against a disk that moved under the plan.
            let table_now = match manual && !disk.is_empty() {
                true => Some(scan.table(&disk)?),
                false => None,
            };
            let table = layout_table(
                &scan,
                &disk,
                layout.as_ref(),
                payload,
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
            fields[ROW_TABLE] = table.field(
                clamp_row(&table.selectable, at),
                !disk.is_empty(),
                std::mem::take(&mut focus_table),
            );
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
                    // The plan carries no picture of the ESP, so the entries
                    // are read from the scan the form drew from. The
                    // confirmation below names every one the install removes.
                    if let Some(held) = answers.layout.as_ref() {
                        let esp = esp::removals(&scan, held);
                        if let Some(layout) = answers.layout.as_mut() {
                            layout.esp = esp;
                        }
                    }
                    if let Some(layout) = answers.layout.as_mut().filter(|layout| {
                        !layout.deletes.is_empty()
                            || !layout.creates.is_empty()
                            || !layout.renames.is_empty()
                    }) {
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
                    focus_table = true;
                    match table.kinds.get(row) {
                        Some(RowKind::Disk(at)) => {
                            let action = item
                                .and_then(|at| table.actions.get(row).and_then(|row| row.get(at)))
                                .copied();
                            match action {
                                Some(PartAction::Create) => {
                                    // The window opens on the free space the
                                    // plan leaves, with the plan's own creates
                                    // placed first. The question is about the
                                    // disk as it stands now.
                                    let (deletes, creates) = match layout.as_ref() {
                                        Some(held) => (held.deletes.clone(), held.creates.clone()),
                                        None => (Vec::new(), Vec::new()),
                                    };
                                    let disk_table = scan.table(at)?;
                                    let names = scan
                                        .get(at)
                                        .map(|entry| slot_names(entry, layout.as_ref()))
                                        .unwrap_or_default();
                                    let rooms = create_rooms(&disk_table, &deletes, &creates);
                                    let holes =
                                        create_holes(&disk_table, &deletes, &creates, &names);
                                    if let Some((offset, gb)) = ask_create(rooms, holes)? {
                                        held_layout(&mut layout, &disk).creates.push(Created {
                                            gb,
                                            offset,
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
                                        // mount, no open and no name.
                                        held.mounts.clear();
                                        held.opens.clear();
                                        held.renames.clear();
                                    }
                                }
                                // A whole-disk pick erases the disk, so the
                                // user is shown what is on it first. A node
                                // alone is not enough to recognise a drive by.
                                // A manual layout already carries the disk's
                                // partitions on the form, so its pick is taken
                                // unasked.
                                _ => {
                                    if manual {
                                        disk = at.clone();
                                    } else {
                                        let confirmed = match scan.get(at) {
                                            Some(entry) => confirm_disk(entry)?,
                                            // A disk row comes from the scan, so
                                            // this arm never runs. A miss has no
                                            // picture to show and takes the disk
                                            // as the row did.
                                            None => true,
                                        };
                                        if confirmed {
                                            disk = at.clone();
                                        }
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
                                    // formats. `short_of` refuses a manual
                                    // container with `CUSTOM_LUKS_LATER`.
                                    // That rule reads `mounts`. A create is
                                    // not a mount, so offering `luks` here
                                    // would be a dead end with no refusal on
                                    // screen.
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
                                // A planned partition is named from its mount
                                // point until the user types a name, which
                                // the cut writes into the script.
                                PartAction::Rename => {
                                    let Some(create) =
                                        layout.as_ref().and_then(|held| held.creates.get(index))
                                    else {
                                        continue;
                                    };
                                    let default = rename_default(&create.target, &create.label);
                                    if let Some(label) = ask_rename("", &default)? {
                                        if let Some(create) = layout
                                            .as_mut()
                                            .and_then(|held| held.creates.get_mut(index))
                                        {
                                            create.label = label;
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
                                            _ => place_target(
                                                &mut layout,
                                                &disk,
                                                partition,
                                                target,
                                                &payload.filesystem,
                                            ),
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
                                        layout
                                            .renames
                                            .retain(|rename| rename.partition != partition.device);
                                        // Reset is the one item a partition
                                        // planned away still offers, so it is
                                        // also what puts it back.
                                        layout.deletes.retain(|at| at != &partition.device);
                                    }
                                }
                                // A renamed partition keeps its other answers.
                                // The name is written by the cut rather than
                                // dropped, so `Reset` is what takes it back.
                                PartAction::Rename => {
                                    let current = layout
                                        .as_ref()
                                        .and_then(|held| held.renamed(&partition.device))
                                        .unwrap_or(&partition.label)
                                        .to_string();
                                    if let Some(label) = ask_rename(&partition.device, &current)? {
                                        let held = held_layout(&mut layout, &disk);
                                        held.renames
                                            .retain(|rename| rename.partition != partition.device);
                                        // A name equal to the label the
                                        // partition already carries states
                                        // nothing, so the plan drops it.
                                        if label != partition.label {
                                            held.renames.push(Rename {
                                                partition: partition.device.clone(),
                                                label,
                                            });
                                        }
                                    }
                                }
                                // A partition planned away loses its answers.
                                // The install cannot mount, format, open or
                                // rename a partition that will not be there.
                                PartAction::Delete => {
                                    let device = partition.device.clone();
                                    let held = held_layout(&mut layout, &disk);
                                    held.mounts.retain(|mount| mount.partition != device);
                                    held.opens.retain(|open| open.partition != device);
                                    held.renames.retain(|rename| rename.partition != device);
                                    if !held.deletes.contains(&device) {
                                        held.deletes.push(device);
                                    }
                                }
                                PartAction::Open => {
                                    let found = scan
                                        .get(&disk)
                                        .map(|entry| entry.keys.clone())
                                        .unwrap_or_default();
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
                        fields = Self::seeded(payload, Given::default(), prompt)?
                            .fields(payload, &scan, &disk, None)
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
                        fields = Self::seeded(payload, Given::default(), prompt)?
                            .fields(payload, &scan, &disk, None)
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
            let disk = ask_disk(given.disk, None, prompt)?;
            // A headless run writes the disk the flags name and draws no
            // picture, so a disk whose partitions cannot be read stops here
            // rather than under the write.
            partitions(&disk)?;
            return Ok(Self {
                disk,
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
                layout: None,
                opened: Opened::Keep,
            });
        }
        Ok(Self {
            disk: given.disk.unwrap_or_default(),
            // The owner decided 2026-09-24 that the hostname starts blank. The
            // payload's own name is not what the machine must be called.
            hostname: given.hostname.unwrap_or_default(),
            user: given.user.unwrap_or_default(),
            password: given.password.unwrap_or_default(),
            encryption: Encryption {
                // `named` refuses a flag naming a kind outside `KINDS`.
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
        scan: &Scan,
        disk: &str,
        layout: Option<&CustomLayout>,
    ) -> Vec<common::ui::Field> {
        use common::ui::Field;
        let table = layout_table(scan, disk, layout, payload, self.encryption.kind != NONE);
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
            table.field(0, !disk.is_empty(), false),
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
                // An ESP entry that boots a system the plan rewrites goes with
                // the partitions above it, and the image writes its own entry
                // afterwards.
                rows.extend(layout.esp.iter().flat_map(|removal| {
                    removal.entries.iter().map(|entry| {
                        (
                            copy::ROW_REMOVED.to_string(),
                            copy::summary_esp(entry, &removal.partition),
                        )
                    })
                }));
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
                    let how = at_boot(open, self.opened);
                    let said = match open.target.as_str() {
                        "/" => copy::opened_root(&open.partition, &payload.filesystem, how),
                        _ => copy::opened(&open.partition, how),
                    };
                    (open.target.clone(), said)
                }));
                if let Some(chain) = copy::boot_chain(&payload.boot) {
                    rows.push(("boot chain".to_string(), chain.to_string()));
                }
            }
            None => rows.extend(copy::written_over(&payload.bootloader, &payload.filesystem)),
        }
        rows
    }
}
