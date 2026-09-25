use super::*;

/// Names one row of the layout table. A row is a whole disk, one partition of
/// that disk, one the plan cuts, or a row the cursor cannot rest on.
#[derive(Clone)]
pub(crate) enum RowKind {
    Disk(String),
    Part {
        index: usize,
    },
    /// Indexes the layout's own `creates`. A planned partition is on no disk
    /// yet, so the machine's partition listing does not hold it.
    Created {
        index: usize,
    },
    /// A row the user reads and cannot answer: the automatic plan's rows and
    /// the systems a partition carries.
    Preview,
}

/// Names the answers one partition's popup gives.
#[derive(Clone, Copy)]
pub(crate) enum PartAction {
    Assign,
    Format,
    Reset,
    Open,
    Close,
    /// Changes the partition table itself. `run` enacts every one of these
    /// through `sfdisk` before fisherman is handed the recipe: `Delete` and
    /// `Clear` plan a removal, `Create` plans a cut whose name rides in the
    /// script, and `Rename` writes an existing partition's label.
    Delete,
    Clear,
    Create,
    Rename,
}

/// Gives the filesystem one partition's answer leaves on the disk. A Format
/// answer wins. Without one the row keeps the filesystem lsblk reported.
pub(crate) fn effective_fs(partition: &Partition, answer: Option<&Mounted>) -> String {
    match answer {
        Some(mounted) if mounted.fstype != OPEN && mounted.fstype != "unformatted" => {
            mounted.fstype.clone()
        }
        _ => partition.fstype.clone(),
    }
}

/// Reports whether a filesystem can carry a mount point. Only FAT carries
/// `/boot/efi` and only swap carries `/swap`. Every other mount point needs
/// one of `LINUX_FILESYSTEMS`.
pub(crate) fn fits(target: &str, fstype: &str) -> bool {
    match target {
        "/boot/efi" => is_fat(fstype),
        "/swap" => fstype == "swap",
        _ => LINUX_FILESYSTEMS.contains(&fstype),
    }
}

/// Lists the mount points one partition can take. A partition is never
/// offered a mount point its filesystem cannot carry. `boot` allows a separate
/// `/boot`, which is an answer only where the target's bootloader can read one,
/// and `composefs` leaves `/var` out because bootc binds it before any fstab
/// unit runs. A point under `/var` still mounts.
pub(crate) fn assigns(
    partition: &Partition,
    answer: Option<&Mounted>,
    boot: bool,
    composefs: bool,
) -> Vec<&'static str> {
    let holds = effective_fs(partition, answer);
    copy::mount_points()
        .into_iter()
        .filter(|target| *target != "/boot" || boot)
        .filter(|target| *target != "/var" || !composefs)
        .filter(|target| fits(target, &holds))
        .collect()
}

/// Lists the mount points an open container takes. A closed container is
/// offered none, because what is inside stays unknown until the user gives
/// the key. `/boot/efi` stays out, because the firmware reads the ESP before
/// anything opens the container, and `composefs` leaves `/var` out as
/// `assigns` does.
pub(crate) fn open_points(composefs: bool) -> Vec<&'static str> {
    ["/", "/var", "/var/home"]
        .into_iter()
        .filter(|target| *target != "/var" || !composefs)
        .collect()
}

/// Builds the Assign submenu from the mount points a partition can take.
/// `UNASSIGN` joins them, so the user can take a mount point back.
fn assign_children(points: &[&'static str]) -> Vec<&'static str> {
    let mut children = points.to_vec();
    children.push(copy::UNASSIGN);
    children
}

/// Names the label a mount point derives, in the words the automatic plan
/// already writes onto the partitions it cuts. A mount point this installer
/// does not name returns empty, and the rename window then opens blank.
pub(crate) fn mount_name(target: &str) -> &'static str {
    match target {
        "/boot/efi" => "EFI-SYSTEM",
        "/boot" => "boot",
        "/" => "root",
        "/var" | "/var/home" => "var",
        _ => "",
    }
}

/// Gives the name the rename window opens on. A label the plan already
/// carries wins, so a second visit keeps the user's own answer. Otherwise a
/// planned partition is named from its mount point and a partition that
/// exists from its own label.
pub(crate) fn rename_default(target: &str, label: &str) -> String {
    match label.is_empty() {
        true => mount_name(target).to_string(),
        false => label.to_string(),
    }
}

/// Takes one partition's mount point away and leaves its format alone. The
/// layout is half answered until the user assigns a point again, and
/// `layout_short_of` blocks `Install` while it is.
pub(crate) fn place_unassign(held: &mut Option<CustomLayout>, partition: &Partition) {
    if let Some(layout) = held.as_mut() {
        if let Some(open) = layout
            .opens
            .iter_mut()
            .find(|open| open.partition == partition.device)
        {
            open.target.clear();
        }
        if let Some(mount) = layout
            .mounts
            .iter_mut()
            .find(|mount| mount.partition == partition.device)
        {
            mount.target.clear();
        }
        // An answer with no mount point and no format states nothing, so the
        // layout drops that mount row.
        layout.mounts.retain(|mount| {
            !(mount.partition == partition.device
                && mount.target.is_empty()
                && mount.fstype == "unformatted")
        });
    }
}

/// Gives the held layout and builds it on the first answer about the disk.
pub(crate) fn held_layout<'a>(
    held: &'a mut Option<CustomLayout>,
    disk: &str,
) -> &'a mut CustomLayout {
    held.get_or_insert_with(|| CustomLayout::empty(disk))
}

/// Points one partition's answer at a mount point. An open container keeps
/// the key it was opened with. Every other partition becomes a mount that
/// stays `unformatted` until a Format answer rewrites it.
pub(crate) fn place_target(
    held: &mut Option<CustomLayout>,
    disk: &str,
    partition: &Partition,
    target: &str,
) {
    let layout = held_layout(held, disk);
    if let Some(open) = layout
        .opens
        .iter_mut()
        .find(|open| open.partition == partition.device)
    {
        open.target = target.to_string();
        return;
    }
    match layout
        .mounts
        .iter_mut()
        .find(|mount| mount.partition == partition.device)
    {
        Some(mount) => mount.target = target.to_string(),
        None => layout.mounts.push(CustomMount {
            partition: partition.device.clone(),
            target: target.to_string(),
            fstype: "unformatted".to_string(),
            passphrase: String::new(),
        }),
    }
}

/// Chooses the filesystem a partition is rewritten to. A format erases a
/// container, so the layout drops that partition's open. `Do not format`
/// returns the answer to `unformatted`. It also drops the mount row where the
/// partition holds no mount point.
pub(crate) fn place_format(
    held: &mut Option<CustomLayout>,
    disk: &str,
    partition: &Partition,
    fstype: &str,
) {
    let keep = fstype == copy::NO_FORMAT;
    let layout = held_layout(held, disk);
    if !keep {
        layout
            .opens
            .retain(|open| open.partition != partition.device);
    }
    let answer = match keep {
        true => "unformatted",
        false => fstype,
    };
    match layout
        .mounts
        .iter_mut()
        .find(|mount| mount.partition == partition.device)
    {
        Some(mount) => mount.fstype = answer.to_string(),
        None => layout.mounts.push(CustomMount {
            partition: partition.device.clone(),
            target: String::new(),
            fstype: answer.to_string(),
            passphrase: String::new(),
        }),
    }
    // The chosen filesystem overrules the mount point. A point the new
    // filesystem cannot carry is cleared here, so no answer holds a pair
    // `fits` refuses.
    if let Some(at) = layout
        .mounts
        .iter()
        .position(|mount| mount.partition == partition.device)
    {
        let (target, fstype) = (
            layout.mounts[at].target.clone(),
            layout.mounts[at].fstype.clone(),
        );
        let carries = match keep {
            true => partition.fstype.as_str(),
            false => fstype.as_str(),
        };
        if !target.is_empty() && !fits(&target, carries) {
            layout.mounts[at].target.clear();
        }
    }
    if keep {
        layout
            .mounts
            .retain(|mount| !(mount.partition == partition.device && mount.target.is_empty()));
    }
}

/// Holds the table under the `partition layout` row. It carries every disk.
/// Under the chosen disk it carries the automatic plan as grey rows, or the
/// disk's own partitions with the layout answers read back onto them.
#[derive(Default)]
pub(crate) struct TableData {
    pub(crate) rows: Vec<Vec<common::ui::Cell>>,
    pub(crate) selectable: Vec<bool>,
    pub(crate) menus: Vec<Vec<common::ui::MenuItem>>,
    pub(crate) actions: Vec<Vec<PartAction>>,
    pub(crate) kinds: Vec<RowKind>,
}

impl TableData {
    pub(crate) fn push(
        &mut self,
        cells: Vec<common::ui::Cell>,
        selectable: bool,
        menus: Vec<common::ui::MenuItem>,
        actions: Vec<PartAction>,
        kind: RowKind,
    ) {
        self.rows.push(cells);
        self.selectable.push(selectable);
        self.menus.push(menus);
        self.actions.push(actions);
        self.kinds.push(kind);
    }

    /// Wraps the table as the form field the user moves the cursor over.
    /// `chosen` drives the status glyph beside the `disk selection` label, and
    /// `focused` opens the table with its own cursor for a caller that just
    /// answered a table row.
    pub(crate) fn field(&self, cursor: usize, chosen: bool, focused: bool) -> common::ui::Field {
        common::ui::Field::table(
            copy::DISK_SELECTION,
            copy::ROW_DISK,
            &copy::layout_headings(),
            &copy::layout_widths(),
            self.rows.clone(),
            self.selectable.clone(),
            self.menus.clone(),
            cursor,
            focused,
            chosen,
        )
    }
}

pub(crate) fn layout_table(
    scan: &Scan,
    disk: &str,
    layout: Option<&CustomLayout>,
    payload: &Payload,
    var_size: &str,
    home: bool,
    encrypted: bool,
) -> TableData {
    let mut table = TableData::default();
    // A systemd-boot target reads its kernel from the ESP. A separate `/boot`
    // would land where the firmware never looks, so the Assign list leaves it
    // out. The automatic plan already cuts no `/boot` row for that target.
    let boot = payload.bootloader != "systemd";
    for entry in &scan.disks {
        let chosen = entry.device == disk;
        // `disks` joins the size, the model and the removable tag with two
        // spaces. The size splits off here and the removable tag is stripped
        // off the model. `strip_suffix` yields a model only where the tag is
        // there, so a disk with no removable tag is named by its node alone.
        let (size, rest) = match entry.detail.split_once("  ") {
            Some((size, rest)) => (size.to_string(), rest),
            None => (entry.detail.clone(), ""),
        };
        let model = rest
            .strip_suffix(copy::REMOVABLE)
            .map(str::trim_end)
            .filter(|model| !model.is_empty());
        // The user recognises a drive by its model. The node tells two
        // identical drives apart, so it rides in brackets after the model.
        // The model column has no heading, and `clipped` ends a long model
        // with `...` where the column runs out.
        let name = match model {
            Some(model) => format!("{model} ({})", entry.device),
            None => entry.device.clone(),
        };
        // The disk rows are one choice among disks, so each carries a radio
        // glyph. The chosen disk is filled in and drawn white.
        let radio = match chosen {
            true => "\u{25c9}",
            false => "\u{25cb}",
        };
        let choice = format!("{radio} {name}");
        let mut row = vec![
            common::ui::Cell::new(&choice),
            common::ui::Cell::new(&size),
            common::ui::Cell::new(""),
            common::ui::Cell::new(""),
            common::ui::Cell::new(""),
            common::ui::Cell::new(""),
        ];
        if chosen {
            row[0] = common::ui::Cell::set(&choice);
        }
        let (menus, actions) = match (layout.is_some(), chosen) {
            (true, true) => disk_menu(entry.partitions.len()),
            _ => (Vec::new(), Vec::new()),
        };
        table.push(
            row,
            true,
            menus,
            actions,
            RowKind::Disk(entry.device.clone()),
        );
        match layout {
            // An automatic layout replaces the chosen disk's partitions with
            // the plan fisherman would cut. The rows are grey and
            // unselectable, because the user answers nothing on them.
            None if chosen => {
                // `automatic_rows` already wrote the tree glyphs into each
                // name, so no glyph is added here.
                for (name, size, filesystem, kind, mount) in
                    automatic_rows(payload, var_size, size_gb(&size), home, encrypted)
                {
                    table.push(
                        vec![
                            common::ui::Cell::new(&name),
                            common::ui::Cell::new(&size),
                            common::ui::Cell::new(&filesystem),
                            // A plan row is not a format answer, so the format
                            // column stays empty. The kind names the type the
                            // cut will write.
                            common::ui::Cell::new(""),
                            common::ui::Cell::new(&kind),
                            common::ui::Cell::new(&mount),
                        ],
                        false,
                        Vec::new(),
                        Vec::new(),
                        RowKind::Preview,
                    );
                }
            }
            // A manual layout draws what is on every disk. Only the chosen
            // disk's partitions carry popups and answers.
            Some(held) => {
                partition_rows(&mut table, entry, chosen.then_some(held), boot, payload);
                // The planned cuts are drawn under the partitions already
                // there, each on the node `created_devices` predicts. Only the
                // chosen disk holds creates, because a create is planned onto
                // the disk the user had chosen when they asked for it.
                if chosen {
                    let devices = created_devices(
                        &entry.device,
                        &drawn_slots(&entry.partitions, &held.deletes),
                        held.creates.len(),
                    );
                    let last = held.creates.len().saturating_sub(1);
                    for (at, create) in held.creates.iter().enumerate() {
                        let branch = match at == last {
                            true => "\u{2514}\u{2500} ",
                            false => "\u{251c}\u{2500} ",
                        };
                        let device = devices.get(at).cloned().unwrap_or_default();
                        let (menus, actions) = created_menu(create, boot, payload.composefs);
                        table.push(
                            created_cells(create, &device, branch),
                            true,
                            menus,
                            actions,
                            RowKind::Created { index: at },
                        );
                    }
                }
            }
            // An automatic layout draws every other disk as it stands today,
            // so the user answers which disk to take before taking it.
            None => partition_rows(&mut table, entry, None, boot, payload),
        }
    }
    table
}

/// Draws one disk's existing partitions. `held` carries the layout's answers
/// where this is the chosen disk; without it every row is grey and opens no
/// popup. The systems a partition carries draw under it as child rows.
fn partition_rows(
    table: &mut TableData,
    disk: &DiskScan,
    held: Option<&CustomLayout>,
    boot: bool,
    payload: &Payload,
) {
    let parts = &disk.partitions;
    let chosen = held.is_some();
    // A planned partition is drawn under the existing ones, so it owns the
    // corner glyph. `usize::MAX` holds every existing partition on the tee
    // glyph while any create is planned.
    let last = match chosen && !held.is_some_and(|held| held.creates.is_empty()) {
        true => usize::MAX,
        false => parts.len().saturating_sub(1),
    };
    for (at, partition) in parts.iter().enumerate() {
        let branch = match at == last {
            true => "\u{2514}\u{2500} ",
            false => "\u{251c}\u{2500} ",
        };
        let answer = held.and_then(|held| held.answer(partition));
        // A partition the plan formats vfat takes the ESP role. The
        // filesystem the answer leaves is the one this test reads.
        let carries = effective_fs(partition, answer.as_ref());
        let deleted = held.is_some_and(|held| held.deletes.contains(&partition.device));
        let renamed = held.and_then(|held| held.renamed(&partition.device));
        let cells = partition_cells(partition, answer, branch, deleted, renamed);
        let (menus, actions) = match held {
            Some(held) => partition_menu(partition, Some(held), boot, payload.composefs),
            None => (Vec::new(), Vec::new()),
        };
        table.push(cells, chosen, menus, actions, RowKind::Part { index: at });
        // A system the partition carries is a child of its row, the way a
        // container's partitions are children of the container's own row.
        // The plan's ESP carries the entries this image writes, and an entry
        // whose system the plan takes gives way to them.
        let labels: Vec<&Label> = disk
            .labels
            .iter()
            .filter(|(device, _)| device == &partition.device)
            .flat_map(|(_, labels)| labels)
            .collect();
        let shown: Vec<(String, bool)> = match held.filter(|held| {
            is_fat(&carries)
                && held
                    .answer(partition)
                    .is_some_and(|mounted| mounted.target == "/boot/efi")
        }) {
            Some(held) => {
                let entries = payload.esp_entries();
                // A format erases the entries the walk found, so the plan
                // draws only what the image writes.
                let mut rows: Vec<(String, bool)> = match held.formatted(&partition.device) {
                    true => Vec::new(),
                    false => labels
                        .iter()
                        .filter(|label| !held.entry_replaced(&label.link))
                        .filter(|label| {
                            !entries
                                .iter()
                                .any(|entry| entry.eq_ignore_ascii_case(&label.name))
                        })
                        .map(|label| (label.name.clone(), false))
                        .collect(),
                };
                rows.extend(entries.iter().map(|entry| (entry.clone(), true)));
                rows.sort_by(|one, other| one.0.cmp(&other.0));
                rows
            }
            None => labels
                .iter()
                .map(|label| (label.name.clone(), false))
                .collect(),
        };
        for (at, (label, written)) in shown.iter().enumerate() {
            let branch = match at + 1 == shown.len() {
                true => "   \u{2514}\u{2500} ",
                false => "   \u{251c}\u{2500} ",
            };
            let said = format!("{branch}{label}");
            let name = match written {
                // The image writes this entry during the install, so the row
                // is an answer rather than a system the disk carries now.
                true => common::ui::Cell::set(&said),
                false => common::ui::Cell::new(&said),
            };
            table.push(
                vec![
                    name,
                    common::ui::Cell::new(""),
                    common::ui::Cell::new(""),
                    common::ui::Cell::new(""),
                    common::ui::Cell::new(""),
                    common::ui::Cell::new(""),
                ],
                false,
                Vec::new(),
                Vec::new(),
                RowKind::Preview,
            );
        }
    }
}

/// Reads the leading number of a size string as whole GB. `"68.7 GB"` gives
/// 68 and `"64G"` gives 64. A string with no leading number gives `None`.
pub(crate) fn size_gb(text: &str) -> Option<u64> {
    text.chars()
        .take_while(|letter| letter.is_ascii_digit() || *letter == '.')
        .collect::<String>()
        .parse::<f64>()
        .ok()
        .map(|number| number as u64)
}

/// Builds the automatic plan, one row per partition fisherman would cut. The
/// ESP is always cut. A non-systemd bootloader adds a `/boot`, and both stay
/// outside any container. The root follows, inside the LUKS container where
/// the answer encrypts it. `home` asks for a home partition of its own, cut
/// out of the container. The home row is drawn as soon as the user chooses
/// it and before its size is typed, so the table shows what the answer did.
/// Sizes come from the disk where its size is known. The root row reads `the
/// rest` where the disk size is unknown, where the arithmetic underflows, and
/// where a separate home is asked for with no size typed yet.
pub(crate) fn automatic_rows(
    payload: &Payload,
    var_size: &str,
    disk: Option<u64>,
    home: bool,
    encrypted: bool,
) -> Vec<(String, String, String, String, String)> {
    let filesystem = payload.filesystem.clone();
    // The ESP costs 2 GB and a non-systemd bootloader's `/boot` costs 2 more.
    // What the disk leaves after them becomes the root or its container.
    let boot = payload.bootloader != "systemd";
    let left = disk.and_then(|disk| disk.checked_sub(2 + if boot { 2 } else { 0 }));
    // `home` gates the home row alone. The root row shrinks by whatever
    // `var_size` holds, whether or not a home row is drawn, so every caller
    // blanks `var_size` when the user shares the root. `collect.rs` does it
    // where it builds the size it passes in.
    let separate = home;
    let home_size = match var_size.is_empty() {
        true => None,
        false => size_gb(var_size),
    };
    // The root takes what the disk leaves after the ESP and `/boot`, less any
    // home size the user typed. A separate home with no size typed leaves the
    // root size unknown.
    let root_size = match home_size {
        Some(home) => left.and_then(|left| left.checked_sub(home)),
        None if separate => None,
        None => left,
    };
    let said = |size: Option<u64>| match size {
        Some(size) => copy::size_said(&size.to_string()),
        None => "the rest".to_string(),
    };
    // The sixth field marks a row inside the LUKS container. The glyph pass
    // below reads it and drops it, so the calling command never sees it.
    // The rows carry the names and types the cut will write, so the plan and
    // the disk read the same afterwards. `efi`, `linux` and `luks` head the
    // type column.
    let mut rows: Vec<(String, String, String, String, String, bool)> = vec![(
        "EFI-SYSTEM".to_string(),
        copy::size_said("2"),
        "fat32".to_string(),
        "efi".to_string(),
        "/boot/efi".to_string(),
        false,
    )];
    if boot {
        rows.push((
            "boot".to_string(),
            copy::size_said("2"),
            "ext4".to_string(),
            "linux".to_string(),
            "/boot".to_string(),
            false,
        ));
    }
    let root = |inside| {
        (
            "root".to_string(),
            said(root_size),
            filesystem.clone(),
            "linux".to_string(),
            "/".to_string(),
            inside,
        )
    };
    let home = |inside| {
        (
            "var".to_string(),
            match home_size {
                Some(size) => copy::size_said(&size.to_string()),
                None => copy::size_said(var_size),
            },
            filesystem.clone(),
            "linux".to_string(),
            "/var/home".to_string(),
            inside,
        )
    };
    if encrypted {
        rows.push((
            "root (LUKS)".to_string(),
            said(left),
            String::new(),
            "luks".to_string(),
            String::new(),
            false,
        ));
        rows.push(root(true));
        if separate {
            rows.push(home(true));
        }
    } else {
        rows.push(root(false));
        if separate {
            rows.push(home(false));
        }
    }
    // The last row of each level takes the corner glyph. A row inside the
    // container indents three columns, under the space the container's own
    // corner glyph leaves.
    let last = |inside: bool| rows.iter().rposition(|(.., at)| *at == inside).unwrap_or(0);
    let (outside, inner) = (last(false), last(true));
    rows.into_iter()
        .enumerate()
        .map(|(at, (name, size, filesystem, kind, mount, inside))| {
            let glyph = match (inside, at == if inside { inner } else { outside }) {
                (false, true) => "\u{2514}\u{2500} ",
                (false, false) => "\u{251c}\u{2500} ",
                (true, true) => "   \u{2514}\u{2500} ",
                (true, false) => "   \u{251c}\u{2500} ",
            };
            (format!("{glyph}{name}"), size, filesystem, kind, mount)
        })
        .collect()
}

/// Draws one existing partition as a table row. Every cell the layout answers
/// is drawn white. `renamed` holds the label the plan writes, which stands in
/// for the label the partition carries now.
fn partition_cells(
    partition: &Partition,
    answer: Option<Mounted>,
    branch: &str,
    deleted: bool,
    renamed: Option<&str>,
) -> Vec<common::ui::Cell> {
    // A partition the plan removes says `remove` in the format column and
    // nothing else. Its filesystem, type and mount stop being true the moment
    // the cut runs.
    if deleted {
        let said = match partition.label.is_empty() {
            true => partition.device.clone(),
            false => format!("{} ({})", partition.label, partition.device),
        };
        return vec![
            common::ui::Cell::set(&format!("{branch}{said}")),
            common::ui::Cell::new(&copy::size_said(&partition.size)),
            common::ui::Cell::new(""),
            common::ui::Cell::set(copy::CELL_REMOVE),
            common::ui::Cell::new(""),
            common::ui::Cell::new(""),
        ];
    }
    let rewritten = answer
        .as_ref()
        .is_some_and(|mounted| mounted.fstype != OPEN && mounted.fstype != "unformatted");
    let filesystem = match &answer {
        Some(mounted) if mounted.fstype == OPEN => common::ui::Cell::set(copy::LUKS_OPEN),
        Some(mounted) if mounted.fstype != "unformatted" => common::ui::Cell::set(&mounted.fstype),
        _ if partition.fstype == "crypto_LUKS" => common::ui::Cell::new(copy::LUKS_CLOSED),
        _ => common::ui::Cell::new(&partition.fstype),
    };
    let format = match rewritten {
        true => common::ui::Cell::set(copy::FORMAT_TICK),
        false => common::ui::Cell::new(""),
    };
    // The type column follows the answer where there is one, so a partition
    // assigned `/boot/efi` reads `efi` before its GPT type says so.
    let kind = match answer
        .as_ref()
        .is_some_and(|mounted| mounted.target == "/boot/efi")
    {
        true => common::ui::Cell::set(copy::EFI_CELL),
        false => common::ui::Cell::new(partition_type_said(&partition.parttype)),
    };
    let mount = match &answer {
        Some(mounted) => common::ui::Cell::set(&mounted.target),
        None => common::ui::Cell::new(""),
    };
    // A labelled partition is named by its label and an unlabelled one by
    // its node. A rename the plan carries stands in for the label the
    // partition holds now. The name cell turns white once the partition
    // carries an answer, so one glance down the first column finds the
    // answered rows.
    let label = renamed.unwrap_or(&partition.label);
    let said = match label.is_empty() {
        true => partition.device.clone(),
        false => format!("{} ({})", label, partition.device),
    };
    let answered = answer.is_some() || renamed.is_some();
    let device = match answered {
        true => common::ui::Cell::set(&format!("{branch}{said}")),
        false => common::ui::Cell::new(&format!("{branch}{said}")),
    };
    vec![
        device,
        common::ui::Cell::new(&copy::size_said(&partition.size)),
        filesystem,
        format,
        kind,
        mount,
    ]
}

/// Builds one existing partition's popup and the action each item answers
/// with. Building the popup writes nothing to the disk. `boot` and `composefs`
/// shape the Assign list as `assigns` does.
pub(crate) fn partition_menu(
    partition: &Partition,
    held: Option<&CustomLayout>,
    boot: bool,
    composefs: bool,
) -> (Vec<common::ui::MenuItem>, Vec<PartAction>) {
    // A partition the plan will remove offers `Reset` alone. Nothing can be
    // mounted, formatted or opened on a partition that will not be there, and
    // `Reset` puts it back.
    if held.is_some_and(|layout| layout.deletes.contains(&partition.device)) {
        return (
            vec![common::ui::MenuItem::new(copy::RESET_CHANGES)],
            vec![PartAction::Reset],
        );
    }
    let answer = held.and_then(|layout| layout.answer(partition));
    let renamed = held.is_some_and(|layout| layout.renamed(&partition.device).is_some());
    let opened = held.is_some_and(|layout| {
        layout
            .opens
            .iter()
            .any(|open| open.partition == partition.device)
    });
    let mut items = Vec::new();
    let mut actions = Vec::new();
    // The layout answer decides this, and lsblk's report only fills the gap.
    // A container the user answered with a plain Format counts as a plain
    // filesystem here and loses its container items.
    let container = match &answer {
        Some(mounted) if mounted.fstype == OPEN => true,
        Some(mounted) if mounted.fstype != "unformatted" => false,
        _ => partition.fstype == "crypto_LUKS",
    };
    if container {
        if opened {
            let points = assign_children(&open_points(composefs));
            items.push(common::ui::MenuItem::under(copy::ASSIGN, &points));
            actions.push(PartAction::Assign);
        }
        items.push(common::ui::MenuItem::new(copy::FORMAT_ROW));
        actions.push(PartAction::Format);
        match opened {
            true => {
                items.push(common::ui::MenuItem::new(copy::CLOSE_ENCRYPTED));
                actions.push(PartAction::Close);
            }
            false => {
                items.push(common::ui::MenuItem::new(copy::OPEN_ENCRYPTED));
                actions.push(PartAction::Open);
            }
        }
    } else {
        let points = assigns(partition, answer.as_ref(), boot, composefs);
        if !points.is_empty() {
            let points = assign_children(&points);
            items.push(common::ui::MenuItem::under(copy::ASSIGN, &points));
            actions.push(PartAction::Assign);
        }
        items.push(common::ui::MenuItem::new(copy::FORMAT_ROW));
        actions.push(PartAction::Format);
    }
    // The name is not an erase, so `Rename` sits above the `Reset` that undoes
    // all the answers and well above `Delete`. The cursor opens on the first
    // item, which must never be the destructive answer.
    items.push(common::ui::MenuItem::new(copy::RENAME_PART));
    actions.push(PartAction::Rename);
    if answer.is_some() || renamed {
        items.push(common::ui::MenuItem::new(copy::RESET_CHANGES));
        actions.push(PartAction::Reset);
    }
    items.push(common::ui::MenuItem::new(copy::DELETE_PART));
    actions.push(PartAction::Delete);
    (items, actions)
}

/// Builds one planned partition's popup. A partition cut blank has nothing to
/// keep and no container to open, so the popup formats it, assigns it or
/// drops it. `boot` and `composefs` shape the Assign list as `assigns` does.
pub(crate) fn created_menu(
    create: &Created,
    boot: bool,
    composefs: bool,
) -> (Vec<common::ui::MenuItem>, Vec<PartAction>) {
    let points: Vec<&'static str> = copy::mount_points()
        .into_iter()
        // `collect` formats a create from `plain_formats`, which carries no
        // `swap`. A `/swap` point could never be completed, so the popup
        // leaves it out.
        .filter(|target| *target != "/swap")
        .filter(|target| *target != "/boot" || boot)
        .filter(|target| *target != "/var" || !composefs)
        .filter(|target| create.fstype.is_empty() || fits(target, &create.fstype))
        .collect();
    let mut items = Vec::new();
    let mut actions = Vec::new();
    if !points.is_empty() {
        let points = assign_children(&points);
        items.push(common::ui::MenuItem::under(copy::ASSIGN, &points));
        actions.push(PartAction::Assign);
    }
    items.push(common::ui::MenuItem::new(copy::FORMAT_ROW));
    actions.push(PartAction::Format);
    items.push(common::ui::MenuItem::new(copy::RENAME_PART));
    actions.push(PartAction::Rename);
    items.push(common::ui::MenuItem::new(copy::DELETE_PART));
    actions.push(PartAction::Delete);
    (items, actions)
}

/// Builds the chosen disk's own popup in a manual layout. It carries the two
/// answers about the partition table rather than about any one partition. An
/// automatic layout offers neither, because it cuts the whole disk itself. An
/// unchosen disk row stays the plain pick it was. `Clear` appears only where
/// the disk still holds partitions to clear.
pub(crate) fn disk_menu(parts: usize) -> (Vec<common::ui::MenuItem>, Vec<PartAction>) {
    let mut items = vec![common::ui::MenuItem::new(copy::CREATE_PART)];
    let mut actions = vec![PartAction::Create];
    if parts > 0 {
        items.push(common::ui::MenuItem::new(copy::CLEAR_PARTS));
        actions.push(PartAction::Clear);
    }
    (items, actions)
}

/// Draws one planned partition as a table row. The node is the one
/// `created_devices` predicts from the slots that survive the plan's deletes.
/// The format column reads `new`, because a partition that does not exist yet
/// says more than a format tick could. A label the plan carries is drawn
/// ahead of the node, as it is on a partition that already exists.
fn created_cells(create: &Created, device: &str, branch: &str) -> Vec<common::ui::Cell> {
    let filesystem = match create.fstype.is_empty() {
        true => common::ui::Cell::new(""),
        false => common::ui::Cell::set(&create.fstype),
    };
    let kind = match create.target == "/boot/efi" {
        true => common::ui::Cell::set(copy::EFI_CELL),
        false => common::ui::Cell::new(""),
    };
    let mount = match create.target.is_empty() {
        true => common::ui::Cell::new(""),
        false => common::ui::Cell::set(&create.target),
    };
    let said = match create.label.is_empty() {
        true => device.to_string(),
        false => format!("{} ({device})", create.label),
    };
    vec![
        common::ui::Cell::set(&format!("{branch}{said}")),
        common::ui::Cell::new(&copy::size_said(&create.gb.to_string())),
        filesystem,
        common::ui::Cell::set(copy::CELL_NEW),
        kind,
        mount,
    ]
}

/// Moves the table cursor onto a row it can rest on. It keeps `at` where that
/// row is selectable. Otherwise it takes the next selectable row below, and
/// the last selectable row above where `at` sits past the end.
pub(crate) fn clamp_row(selectable: &[bool], at: usize) -> usize {
    let at = at.min(selectable.len().saturating_sub(1));
    if selectable.get(at).copied().unwrap_or(false) {
        return at;
    }
    if let Some(next) = (at + 1..selectable.len()).find(|next| selectable[*next]) {
        return next;
    }
    selectable[..at].iter().rposition(|ok| *ok).unwrap_or(0)
}

/// Names the partition type for the table, from the GPT type GUID or from the
/// byte lsblk prints for a dos label. An unrecognised type reads empty,
/// because a raw GUID tells the user nothing.
fn partition_type_said(parttype: &str) -> &'static str {
    match parttype.to_ascii_lowercase().as_str() {
        "c12a7328-f81f-11d2-ba4b-00a0c93ec93b" | "0xef" => "efi",
        "0fc63daf-8483-4772-8e79-3d69d8477de4" | "0x83" => "linux",
        "e6d6d379-f507-44c2-a23c-238f2a3df928" | "0x8e" => "lvm",
        "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f" | "0x82" => "swap",
        _ => "",
    }
}

/// Reports whether the installed machine can use a key. A key file for a data
/// volume is read at boot from the installed root, so it is usable only where
/// the layout opens a container as `/`. A passphrase is typed at boot and is
/// stored nowhere, so it is always usable.
pub(crate) fn usable_key(key: &Key, encrypted_root: bool) -> bool {
    encrypted_root || matches!(key, Key::Passphrase(_))
}

pub(crate) const LINUX_FILESYSTEMS: [&str; 4] = ["ext3", "ext4", "xfs", "btrfs"];

/// Finds the key one container is opened with. A key a system on the disk
/// already records is reused where the installed machine could use it.
/// Otherwise the installer asks the user.
pub(crate) fn open_key(
    partition: &str,
    held: Option<&CustomLayout>,
    found: &Discovered,
) -> Result<Key, String> {
    let encrypted_root =
        held.is_some_and(|layout| layout.opens.iter().any(|open| open.target == "/"));
    match found
        .key(partition)
        .cloned()
        .filter(|key| usable_key(key, encrypted_root))
    {
        Some(key) => Ok(key),
        None => ask_key(!encrypted_root),
    }
}

/// Asks for one container's key on a screen of its own. `plain_root` marks a
/// layout whose root is not an encrypted container, and `key_methods` drops
/// the key file row for it. Esc returns `INTERRUPTED`, which `collect` reads
/// as a return to the layout table.
fn ask_key(plain_root: bool) -> Result<Key, String> {
    let mut fields = vec![
        common::ui::Field::pick(copy::KEY_TYPE, key_methods(plain_root), Some(0)),
        common::ui::Field::secret(copy::KEY_PASSPHRASE, ""),
        common::ui::Field::text(copy::KEY_FILE_PATH, ""),
    ];
    let actions = [copy::KEY_OPEN, copy::KEY_BACK];
    let mut left = Vec::new();
    // One question about one partition belongs in a small window over the
    // layout table, so the key screen draws in an overlay.
    let filled = common::ui::in_overlay(40, 10, || {
        common::ui::form(
            &mut fields,
            &actions,
            |fields| key_short_of(&fields[0].value(), &fields[1].value(), &fields[2].value()),
            // The key screen holds no confirmation row, so no pair is
            // compared.
            |_| Vec::new(),
            |fields| key_asked(&fields[0].value()),
            copy::INSTALL_KEYS,
            &[],
            &[],
            0,
            &mut left,
        )
    })?;
    match filled {
        common::ui::Filled::Took(0) => Ok(key_of(
            &fields[0].value(),
            &fields[1].value(),
            &fields[2].value(),
        )),
        _ => Err(common::ui::INTERRUPTED.to_string()),
    }
}

/// Shows a disk one last time before a whole-disk install takes it. The
/// screen names the model and the node and lists the partitions the scan
/// found on the disk. `true` is the answer that confirms the disk.
pub(crate) fn confirm_disk(disk: &DiskScan) -> Result<bool, String> {
    let (size, model) = match disk.detail.split_once("  ") {
        Some((size, rest)) => (
            size.to_string(),
            rest.split("  ").next().unwrap_or("").to_string(),
        ),
        None => (disk.detail.clone(), String::new()),
    };
    let heading = match model.is_empty() {
        true => format!("{}  {size}", disk.device),
        false => format!("{model} ({})  {size}", disk.device),
    };
    let rows: Vec<(String, String)> = match disk.partitions.is_empty() {
        true => vec![("no partitions".to_string(), String::new())],
        false => disk
            .partitions
            .iter()
            .map(|part| {
                let name = match part.label.is_empty() {
                    true => part.device.clone(),
                    false => format!("{} ({})", part.label, part.device),
                };
                let detail = match part.fstype.is_empty() {
                    true => copy::size_said(&part.size),
                    false => format!("{}  {}", copy::size_said(&part.size), part.fstype),
                };
                (name, detail)
            })
            .collect(),
    };
    common::ui::confirm_over(
        copy::USE_DISK,
        &heading,
        &rows,
        copy::USE_DISK_ACTION,
        copy::GO_BACK,
        common::ui::SIDE,
    )
}

/// Gives the slot the LUKS container takes in the whole-disk plan. The
/// container takes the root's slot, so the plan is read unencrypted and the
/// row mounted at `/` is counted. The ESP and any `/boot` come before it.
pub(crate) fn container_number(payload: &Payload) -> usize {
    let plan = automatic_rows(payload, "", None, false, false);
    1 + plan
        .iter()
        .position(|(.., mount)| mount.as_str() == "/")
        .unwrap_or(0)
}

/// Picks which of the window's six rows a kind asks for. A kind that owes a
/// PIN shows the PIN pair and skips the passphrase pair, so its rows are not
/// contiguous. An image with no LUKS initramfs asks the kind row alone.
pub(crate) fn luks_window_rows(chosen: &str, luks_initramfs: bool) -> Vec<usize> {
    match () {
        _ if !luks_initramfs => vec![0],
        _ if Encryption::wants_passphrase(chosen) => vec![0, 1, 2, 3],
        _ if Encryption::wants_pin(chosen) => vec![0, 1, 4, 5],
        _ => vec![0],
    }
}

/// Gives the row the window reopens on once a kind is taken, which is the
/// first half of the pair that kind owes. A kind that owes no secret returns
/// `None` and answers the window outright.
pub(crate) fn luks_window_start(chosen: &str) -> Option<usize> {
    match () {
        _ if Encryption::wants_passphrase(chosen) => Some(2),
        _ if Encryption::wants_pin(chosen) => Some(4),
        _ => None,
    }
}

/// Asks for the encryption kind and for the passphrase or PIN it owes. A kind
/// that owes one asks for it twice on the one screen, so the user confirms
/// it. No kind asks for both, because no name in `KINDS` ends in both
/// `passphrase` and `pin`. Esc leaves the kind as it was.
pub(crate) fn edit_luks(
    kind: &str,
    passphrase: &str,
    partition: Option<&str>,
    tpm: bool,
    luks_initramfs: bool,
    with_none: bool,
) -> Result<Option<(String, String, String)>, String> {
    // The gap row holds the kind apart from the secret the container is
    // opened with. `kind_at` finds the held option by its label, because the
    // window that makes a container leaves `none` out and its indices run
    // behind `KINDS`.
    let options = kinds(tpm, luks_initramfs, with_none);
    let at = kind_at(&options, kind);
    let mut fields = vec![
        common::ui::Field::radio(copy::ENCRYPTION_TYPE, options, at),
        common::ui::Field::gap(),
        common::ui::Field::secret(copy::ROW_PASSPHRASE, passphrase),
        common::ui::Field::secret(copy::CONFIRM_PASSPHRASE, ""),
        common::ui::Field::secret(copy::ROW_PIN, ""),
        common::ui::Field::secret(copy::CONFIRM_PIN, ""),
    ];
    // The header names the container the window is for. It stays empty where
    // the partition is unknown.
    let header: Vec<common::ui::HeaderLine> = match partition {
        Some(partition) => vec![common::ui::HeaderLine::Row(format!(
            "{}: {partition}",
            copy::LUKS_PARTITION
        ))],
        None => Vec::new(),
    };
    // `start` reopens the window on the secret row when a kind that owes one
    // is taken. A kind that owes no secret closes the window outright, so the
    // user presses no further key.
    let mut start = 0;
    // `left` survives the window's reopens, so a mismatched pair keeps its
    // red when the user changes the kind and the window redraws.
    let mut left: Vec<usize> = Vec::new();
    loop {
        let filled = common::ui::in_titled_overlay(
            common::ui::WINDOW_WIDTH,
            // 18 rows hold the kind radio, either secret pair and the refusal
            // line under them on a machine whose kinds carry a reason.
            18,
            copy::PARTITION_ENCRYPTION,
            || {
                common::ui::form(
                    &mut fields,
                    &[],
                    |fields| {
                        luks_short_of(
                            written(&fields[0].value()),
                            &fields[2].value(),
                            &fields[3].value(),
                            &fields[4].value(),
                            &fields[5].value(),
                            luks_initramfs,
                        )
                    },
                    |fields| {
                        [pair_mismatch(fields, [2, 3]), pair_mismatch(fields, [4, 5])].concat()
                    },
                    |fields| luks_window_rows(written(&fields[0].value()), luks_initramfs),
                    copy::SUBMIT_KEYS,
                    &header,
                    &[],
                    start,
                    &mut left,
                )
            },
        )?;
        let chosen = written(&fields[0].value()).to_string();
        match filled {
            common::ui::Filled::Changed(0) => match luks_window_start(&chosen) {
                Some(at) => start = at,
                None => return Ok(Some((chosen, String::new(), String::new()))),
            },
            common::ui::Filled::Took(_) => {
                return Ok(Some((
                    chosen.clone(),
                    match Encryption::wants_passphrase(&chosen) {
                        true => fields[2].value(),
                        false => String::new(),
                    },
                    match Encryption::wants_pin(&chosen) {
                        true => fields[4].value(),
                        false => String::new(),
                    },
                )));
            }
            _ => return Ok(None),
        }
    }
}

/// Names each partition on one disk for the neighbour boxes of the create
/// window's bar, by the label the layout table draws. An unlabelled partition
/// takes its short node without the `/dev/` path, because a box holds at most
/// a quarter of the bar. A partition the plan deletes is left out, because a
/// create takes its slot number.
pub(crate) fn slot_names(disk: &DiskScan, held: Option<&CustomLayout>) -> Vec<(usize, String)> {
    let short = |device: &str, label: &str| match label.is_empty() {
        true => device.rsplit('/').next().unwrap_or(device).to_string(),
        false => label.to_string(),
    };
    let mut names: Vec<(usize, String)> = disk
        .partitions
        .iter()
        .filter(|partition| !held.is_some_and(|held| held.deletes.contains(&partition.device)))
        .filter_map(|partition| {
            let label = held
                .and_then(|held| held.renamed(&partition.device))
                .unwrap_or(&partition.label);
            let number = partition_number(&partition.device).ok()?;
            Some((number, short(&partition.device, label)))
        })
        .collect();
    if let Some(held) = held {
        let devices = created_devices(
            &disk.device,
            &drawn_slots(&disk.partitions, &held.deletes),
            held.creates.len(),
        );
        for (device, create) in devices.iter().zip(&held.creates) {
            if let Ok(number) = partition_number(device) {
                names.push((number, short(device, &create.label)));
            }
        }
    }
    names
}

/// Asks where a planned partition goes: how far into the free space it starts
/// and how big it is. `rooms` holds what the plan leaves. The size opens on
/// the first free region's whole GB, which is the hole a delete left, and the
/// offset opens at zero. A size and offset that fit nowhere are refused here,
/// because `sfdisk` accepts an oversized request and shrinks the partition
/// silently once the deletes are written.
pub(crate) fn ask_create(
    rooms: Rooms,
    holes: Vec<common::ui::Hole>,
) -> Result<Option<(u64, u64)>, String> {
    let header = [common::ui::HeaderLine::Placement {
        holes,
        offset: 0,
        size: 1,
        available: copy::ROW_AVAILABLE.to_string(),
    }];
    let mut offset = "0".to_string();
    let mut size = match rooms.first {
        0 => String::new(),
        gb => gb.to_string(),
    };
    let mut refusal = String::new();
    loop {
        // The refusal rides on the next window's label, and the refused values
        // stay in the fields so the user corrects them rather than retyping.
        // A measure field answers with `Changed` on enter before the form's
        // own blocked branch can draw, so `create_short_of` is asked again
        // here and the window opens a second time with its reason.
        let label = match refusal.is_empty() {
            true => copy::ROW_PARTITION_SIZE.to_string(),
            false => refusal.clone(),
        };
        let mut fields = vec![
            common::ui::Field::measure(copy::ROW_OFFSET, &offset, "GB"),
            common::ui::Field::measure(&label, &size, "GB"),
        ];
        let mut left: Vec<usize> = Vec::new();
        let filled = common::ui::in_titled_overlay(
            common::ui::WINDOW_WIDTH,
            // 13 rows hold the placement bar, the available line, a blank
            // after each, the two measure rows, and the room the refusal takes
            // on the size row's label.
            13,
            copy::NEW_PARTITION,
            || {
                common::ui::form(
                    &mut fields,
                    &[],
                    |fields| create_short_of(&fields[0].value(), &fields[1].value(), rooms.largest),
                    |_| Vec::new(),
                    |_| vec![0, 1],
                    copy::SUBMIT_KEYS,
                    &header,
                    &[],
                    // The owner asked for the cursor to open on the size,
                    // because the offset is the rarer answer.
                    1,
                    &mut left,
                )
            },
        )?;
        if !submitted(filled) {
            return Ok(None);
        }
        offset = fields[0].value();
        size = fields[1].value();
        match create_short_of(&offset, &size, rooms.largest) {
            None => {
                return Ok(Some((
                    size_gb(&offset).unwrap_or(0),
                    size_gb(&size).unwrap_or(0),
                )))
            }
            // An empty size field's own refusal is empty, because the blocked
            // button states the screen is unfinished. This window has no
            // action button, so the empty answer gets a reason of its own.
            Some(said) => {
                refusal = match said.is_empty() {
                    true => copy::NEW_SIZE_NEEDED.to_string(),
                    false => said,
                }
            }
        }
    }
}

/// Whether the create window's key was its submit. A measure field answers
/// with `Changed` on enter, because the main form redraws the row it sizes
/// from that key. This window has no row to redraw and no action button to
/// take instead, so the same key is its submit. Any other result, including
/// the `Left` that `esc` twice returns, answers nothing.
pub(crate) fn submitted(filled: common::ui::Filled) -> bool {
    matches!(
        filled,
        common::ui::Filled::Took(_) | common::ui::Filled::Changed(_)
    )
}

/// Says why a placed create cannot be taken. `room` is the largest free
/// region's whole GB, so an offset and a size that fit no region are refused
/// with the most the disk still has. An empty size returns an empty refusal,
/// because the window's own legend says the screen is unfinished.
pub(crate) fn create_short_of(offset: &str, size: &str, room: u64) -> Option<String> {
    let Some(size) = size_gb(size) else {
        return Some(String::new());
    };
    let offset = size_gb(offset).unwrap_or(0);
    match size == 0 || offset.saturating_add(size) > room {
        true => Some(copy::custom_too_big(room)),
        false => None,
    }
}

/// Asks for the name one partition takes. The field opens on the name the
/// plan gives it already, which is the partition's own label or the name its
/// mount point derives. Esc answers nothing, and `collect` then returns to the
/// layout table.
pub(crate) fn ask_rename(partition: &str, held: &str) -> Result<Option<String>, String> {
    let mut fields = vec![common::ui::Field::text(copy::NEW_NAME, held)];
    let actions = [copy::RENAME_PART, copy::GO_BACK];
    let header: Vec<common::ui::HeaderLine> = match partition.is_empty() {
        true => Vec::new(),
        false => vec![common::ui::HeaderLine::Row(partition.to_string())],
    };
    let mut left: Vec<usize> = Vec::new();
    let filled = common::ui::in_overlay(40, 8, || {
        common::ui::form(
            &mut fields,
            &actions,
            |fields| match fields[0].value().is_empty() {
                true => Some(copy::NAME_NEEDED.to_string()),
                false => None,
            },
            |_| Vec::new(),
            |_| vec![0],
            copy::INSTALL_KEYS,
            &header,
            &[],
            0,
            &mut left,
        )
    })?;
    match filled {
        common::ui::Filled::Took(0) => Ok(Some(fields[0].value())),
        _ => Ok(None),
    }
}

/// Lists the ways a container opens. The key file row is dropped where the
/// root is not an encrypted container, because `usable_key` refuses that key
/// and the user is never offered a row they cannot take.
pub(crate) fn key_methods(plain_root: bool) -> Vec<Choice> {
    let mut methods = vec![Choice::new(copy::KEY_PASSPHRASE, "")];
    if !plain_root {
        methods.push(Choice::new(copy::KEY_FILE, ""));
    }
    methods
}

/// Picks the rows the key screen asks. The chosen method decides which of the
/// two secret rows is asked. The screen never asks for both.
fn key_asked(method: &str) -> Vec<usize> {
    match method == copy::KEY_FILE {
        true => vec![0, 2],
        false => vec![0, 1],
    }
}

/// Says what the LUKS type window still needs. A kind other than `none` is
/// refused outright on an image with no LUKS initramfs. A kind that owes
/// neither a passphrase nor a PIN is answered outright. A kind that owes
/// either needs both halves filled and agreeing. The empty confirmation is
/// refused here, because `pair_mismatch` calls an empty half no disagreement
/// and the window would submit a secret the user never confirmed.
pub(crate) fn luks_short_of(
    kind: &str,
    passphrase: &str,
    confirm: &str,
    pin: &str,
    confirm_pin: &str,
    luks_initramfs: bool,
) -> Option<String> {
    if kind != NONE && !luks_initramfs {
        return Some(copy::NO_LUKS_INITRAMFS.to_string());
    }
    if Encryption::wants_passphrase(kind) {
        if passphrase.is_empty() {
            return Some(copy::still_needs(&[copy::ROW_PASSPHRASE]));
        }
        if confirm.is_empty() {
            return Some(copy::still_needs(&[copy::CONFIRM_PASSPHRASE]));
        }
        return match passphrase == confirm {
            true => None,
            // The mismatch is already drawn red on the pair, so the refusal
            // stays empty and blocks the action without repeating itself.
            false => Some(String::new()),
        };
    }
    if Encryption::wants_pin(kind) {
        if pin.is_empty() {
            return Some(copy::still_needs(&[copy::ROW_PIN]));
        }
        if confirm_pin.is_empty() {
            return Some(copy::still_needs(&[copy::CONFIRM_PIN]));
        }
        return match pin == confirm_pin {
            true => None,
            false => Some(String::new()),
        };
    }
    None
}

/// Says why the key screen is unfinished. An empty box returns an empty
/// refusal, because the greyed `Open` button already says so. A path that
/// names no file is stated, because nothing else on the screen would tell the
/// user that.
fn key_short_of(method: &str, passphrase: &str, path: &str) -> Option<String> {
    match method == copy::KEY_FILE {
        true if path.is_empty() => Some(String::new()),
        true if !Path::new(path).is_file() => Some(copy::KEY_FILE_MISSING.to_string()),
        true => None,
        false if passphrase.is_empty() => Some(String::new()),
        false => None,
    }
}

fn key_of(method: &str, passphrase: &str, path: &str) -> Key {
    match method == copy::KEY_FILE {
        true => Key::File(PathBuf::from(path)),
        false => Key::Passphrase(passphrase.to_string()),
    }
}

/// Says what a manual layout still needs before fisherman can take it. The
/// layout needs one root, one ESP, no mount point claimed twice, and no
/// filesystem that cannot carry the point it holds. The editor lets the user
/// build a half answered layout, so `short_of` runs this on every draw and
/// keeps `Install` blocked until it returns `None`.
pub(crate) fn layout_short_of(
    layout: &CustomLayout,
    seals: bool,
    table: Option<&DiskTable>,
    reserve: u64,
) -> Option<String> {
    // A root below the image's reserve holds no installation. The disk would
    // be cut, formatted and handed to fisherman, which then fails for no space
    // with the old partition table already gone. The floor is the reserve the
    // automatic path's home answer already uses.
    if let Some(root) = layout
        .creates
        .iter()
        .find(|create| create.target == "/" && create.gb < reserve)
    {
        return Some(copy::custom_root_too_small(root.gb, reserve));
    }
    // This weighs the whole plan against the room the disk has now. The
    // create window already refused a single oversized create as the user
    // typed it. This catches the plan that grew too big afterwards, where a
    // delete was taken back or where two creates each fit and together do not.
    if let Some(table) = table {
        if let Some(why) =
            table.wrong_label_for(&layout.deletes, layout.creates.len(), layout.renames.len())
        {
            return Some(why);
        }
        if let Err(room) = place_creates(table, &layout.deletes, &layout.creates) {
            return Some(copy::custom_too_big(room));
        }
    }
    // A planned partition arrives blank and has nothing to keep, so its
    // format is checked before its mount point. The refusal then names the
    // missing format instead of calling the partition unmounted.
    if layout.creates.iter().any(|create| create.fstype.is_empty()) {
        return Some(copy::CUSTOM_UNFORMATTED.to_string());
    }
    // Planned partitions answer the same rules as the partitions already
    // there. The targets of the mounts, the opens and the creates are judged
    // as one list.
    let targets: Vec<String> = layout
        .mounts
        .iter()
        .map(|mount| mount.target.clone())
        .chain(layout.opens.iter().map(|open| open.target.clone()))
        .chain(layout.creates.iter().map(|create| create.target.clone()))
        .collect();
    // A format with no mount point and an open container with no mount point
    // are both half answers. The install would write the partition and then
    // not know where it goes.
    if targets.iter().any(String::is_empty) {
        return Some(copy::CUSTOM_UNMOUNTED.to_string());
    }
    if !targets.iter().any(|target| target == "/") {
        return Some(copy::CUSTOM_ROOT.to_string());
    }
    for (at, target) in targets.iter().enumerate() {
        if targets[..at].contains(target) {
            return Some(copy::CUSTOM_DUPLICATE.to_string());
        }
    }
    // bootc installs its bootloader through the target's `/boot/efi`, and
    // fisherman's automatic path always cuts one. A manual layout with no ESP
    // leaves the machine no bootloader once the disk is rewritten.
    if !targets.iter().any(|target| target == "/boot/efi") {
        return Some(copy::CUSTOM_ESP.to_string());
    }
    // fisherman's `customMounts` carries no passphrase field yet, which 8G
    // phase 2 adds. The recipe would be refused at fisherman's own validation
    // after the user confirmed the install, so the editor refuses it here.
    if layout.mounts.iter().any(|mount| mount.fstype == "luks") {
        return Some(copy::CUSTOM_LUKS_LATER.to_string());
    }
    // A composefs-sealed deployment needs fs-verity, which only ext4 and
    // btrfs carry. A manual root on any other filesystem cannot boot it.
    if seals {
        // A planned root is always formatted, so it answers the same
        // filesystem rule as a root the user chose to reformat.
        let formatted = layout
            .mounts
            .iter()
            .filter(|mount| mount.fstype != "unformatted")
            .map(|mount| (mount.target.as_str(), mount.fstype.as_str()))
            .chain(
                layout
                    .creates
                    .iter()
                    .map(|create| (create.target.as_str(), create.fstype.as_str())),
            );
        for (target, fstype) in formatted {
            if target == "/" && !["ext4", "btrfs"].contains(&fstype) {
                return Some(copy::CUSTOM_VERITY.to_string());
            }
        }
    }
    None
}
