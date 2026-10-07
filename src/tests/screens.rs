use super::*;
use common::ui::testing::{ConfirmationFrame, FormFrame, FormMode, Screen, Snapshot};

const WIDTH: u16 = 80;
const HEIGHT: u16 = 50;

fn assert_screen(name: &str, screen: Snapshot) -> String {
    let rendered = screen.to_string();
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots"));
    settings.set_prepend_module_to_snapshot(false);
    settings.set_omit_expression(true);
    settings.bind(|| insta::assert_snapshot!(format!("installer-screen__{name}"), &rendered));
    rendered
}

fn assert_style_at(rendered: &str, x: u16, y: u16, style: &str, what: &str) {
    let coordinate = format!("x: {x}, y: {y},");
    let line = rendered
        .lines()
        .find(|line| line.contains(&coordinate))
        .unwrap_or_else(|| panic!("{what} has no recorded style at {x},{y}"));
    assert!(line.contains(style), "{what} has the wrong style: {line}");
}

fn screen() -> Screen {
    Screen::full_screen(WIDTH, HEIGHT, copy::installing("deb2"))
}

fn form<'a>(
    fields: &'a [common::ui::Field],
    visible: &'a [usize],
    mode: &'a FormMode,
    actions: &'a [&'a str],
    blocked: Option<&'a str>,
    header: &'a [common::ui::HeaderLine],
) -> FormFrame<'a> {
    FormFrame {
        fields,
        visible,
        cursor: 0,
        button: 0,
        mode,
        actions,
        blocked,
        tried: blocked.is_some(),
        keys: copy::INSTALL_KEYS,
        header,
        sections: &[
            (ROW_HOSTNAME, copy::SETUP_OS),
            (ROW_LAYOUT, copy::SETUP_DISK),
        ],
    }
}

fn fixture_scan() -> Scan {
    scan_of(
        &[
            ("/dev/sdb", "16.1 GB  Cruzer Blade  removable"),
            ("/dev/vda", "68.7 GB  QEMU HARDDISK"),
        ],
        &[
            (
                "/dev/vda",
                vec![
                    Partition {
                        device: "/dev/vda1".to_string(),
                        size: "512M".to_string(),
                        fstype: "vfat".to_string(),
                        label: "EFI".to_string(),
                        ..Default::default()
                    },
                    Partition {
                        device: "/dev/vda2".to_string(),
                        size: "63.5G".to_string(),
                        fstype: "ext4".to_string(),
                        label: "old-root".to_string(),
                        ..Default::default()
                    },
                    Partition {
                        device: "/dev/vda3".to_string(),
                        size: "60G".to_string(),
                        fstype: "crypto_LUKS".to_string(),
                        ..Default::default()
                    },
                ],
            ),
            ("/dev/sdb", Vec::new()),
        ],
    )
}

fn answers(complete: bool) -> Answers {
    Answers {
        disk: String::from(if complete { "/dev/vda" } else { "" }),
        hostname: String::from(if complete { "deb2" } else { "" }),
        user: String::from(if complete { "tect" } else { "" }),
        password: String::from(if complete { "hunter2" } else { "" }),
        encryption: Encryption {
            kind: NONE.to_string(),
            passphrase: String::new(),
            pin: String::new(),
        },
        layout: None,
        opened: Opened::Keep,
    }
}

fn fixture_panel(payload: &Payload) -> Vec<common::ui::HeaderLine> {
    panel_lines(
        &crate::firmware::Firmware {
            efivars: true,
            secure_boot: Some(false),
            setup_mode: Some(true),
            platform_key: crate::firmware::PlatformKey::None,
            database: false,
        },
        payload,
    )
}

#[test]
fn the_install_form_snapshots_blocked_and_ready_states() {
    let payload = a_payload();
    let scan = fixture_scan();
    let panel = fixture_panel(&payload);
    let actions = crate::collect::install_actions();
    let mode = FormMode::Rows;

    let blocked_answers = answers(false);
    let blocked_fields = blocked_answers.fields(&payload, &scan, "", None);
    let blocked_visible = asked(&blocked_fields);
    let blocked = short_of(&blocked_fields, "", None, &payload, false, None).unwrap();
    let rendered = assert_screen(
        "install-blocked",
        form(
            &blocked_fields,
            &blocked_visible,
            &mode,
            &actions,
            Some(&blocked),
            &panel,
        )
        .render(&screen()),
    );
    assert_style_at(
        &rendered,
        4,
        40,
        "modifier: DIM",
        "the blocked Install action",
    );

    let ready_answers = answers(true);
    let ready_fields = ready_answers.fields(&payload, &scan, "/dev/vda", None);
    let ready_visible = asked(&ready_fields);
    let rendered = assert_screen(
        "install-ready",
        form(&ready_fields, &ready_visible, &mode, &actions, None, &panel).render(&screen()),
    );
    assert_style_at(
        &rendered,
        4,
        17,
        "fg: Rgb(238, 111, 248), bg: Reset, underline: Reset, modifier: BOLD",
        "the selected hostname row",
    );
}

fn action_name(action: PartAction) -> &'static str {
    match action {
        PartAction::Assign => "assign",
        PartAction::Format => "format",
        PartAction::Reset => "reset",
        PartAction::Open => "open",
        PartAction::Close => "close",
        PartAction::Delete => "delete",
        PartAction::Clear => "clear",
        PartAction::Create => "create",
        PartAction::Rename => "rename",
    }
}

fn menu_field(title: &str, menus: Vec<common::ui::MenuItem>) -> Vec<common::ui::Field> {
    vec![common::ui::Field::table(
        copy::DISK_SELECTION,
        copy::ROW_DISK,
        &copy::layout_headings(),
        &copy::layout_widths(),
        vec![vec![
            common::ui::Cell::set(title),
            common::ui::Cell::new("68.7 GB"),
            common::ui::Cell::new("ext4"),
            common::ui::Cell::new(""),
            common::ui::Cell::new("linux"),
            common::ui::Cell::set("/"),
        ]],
        vec![true],
        vec![menus],
        0,
        true,
        true,
    )]
}

#[test]
fn every_partition_editor_action_has_a_snapshot() {
    let plain = Partition {
        device: "/dev/vda2".to_string(),
        size: "63.5G".to_string(),
        fstype: "ext4".to_string(),
        ..Default::default()
    };
    let encrypted = Partition {
        device: "/dev/vda3".to_string(),
        size: "60G".to_string(),
        fstype: "crypto_LUKS".to_string(),
        ..Default::default()
    };
    let reset = CustomLayout {
        disk: "/dev/vda".to_string(),
        mounts: vec![CustomMount {
            partition: plain.device.clone(),
            target: "/".to_string(),
            fstype: "ext4".to_string(),
            passphrase: String::new(),
        }],
        ..Default::default()
    };
    let opened = CustomLayout {
        disk: "/dev/vda".to_string(),
        opens: vec![LuksOpen {
            partition: encrypted.device.clone(),
            target: "/".to_string(),
            key: Key::Passphrase("secret".to_string()),
        }],
        ..Default::default()
    };
    let sources = [
        ("disk", disk_menu(2)),
        ("partition", partition_menu(&plain, None, true)),
        (
            "changed partition",
            partition_menu(&plain, Some(&reset), true),
        ),
        ("closed container", partition_menu(&encrypted, None, true)),
        (
            "open container",
            partition_menu(&encrypted, Some(&opened), true),
        ),
        ("new partition", created_menu(&Created::default(), true)),
    ];
    let mut covered = Vec::new();
    for (title, (menus, actions)) in sources {
        for (at, action) in actions.iter().copied().enumerate() {
            let name = action_name(action);
            if covered.contains(&name) {
                continue;
            }
            covered.push(name);
            let fields = menu_field(title, menus.clone());
            let visible = [0];
            let mode = FormMode::TableMenu {
                open: None,
                cursor: at,
            };
            let frame = FormFrame {
                fields: &fields,
                visible: &visible,
                cursor: 0,
                button: 0,
                mode: &mode,
                actions: &[],
                blocked: None,
                tried: false,
                keys: copy::INSTALL_KEYS,
                header: &[],
                sections: &[],
            };
            assert_screen(
                &format!("editor-{name}"),
                frame.render(&Screen::inline(WIDTH, 22)),
            );
        }
    }
    covered.sort_unstable();
    assert_eq!(
        covered,
        ["assign", "clear", "close", "create", "delete", "format", "open", "rename", "reset",]
    );
}

#[test]
fn the_confirmation_handoff_and_progress_screens_use_production_copy() {
    let payload = a_payload();
    let answers = answers(true);
    let rows = answers.summary(&payload);
    assert_screen(
        "installation-summary",
        ConfirmationFrame {
            heading: copy::INSTALLATION_SUMMARY,
            note: &copy::erasing(&answers.disk),
            warning: &[],
            rows: &rows,
            ready: copy::READY,
            yes: copy::START_INSTALLATION,
            no: copy::GO_BACK,
            button: 0,
        }
        .render(&screen()),
    );
    let rendered = assert_screen(
        "shell-handoff",
        ConfirmationFrame {
            heading: copy::EXIT_SHELL,
            note: &copy::switch_note(2),
            warning: &[],
            rows: &[],
            ready: copy::EXIT_SHELL,
            yes: copy::EXIT_SHELL,
            no: copy::GO_BACK,
            button: 1,
        }
        .render(&screen()),
    );
    assert_style_at(
        &rendered,
        38,
        27,
        "modifier: BOLD | REVERSED",
        "the selected Go back refusal",
    );

    let notes = vec![
        "Pulling manifest".to_string(),
        "Deploying ghcr.io/tectonic-os/deb2:latest".to_string(),
    ];
    assert_screen(
        "installation-progress",
        screen().progress(
            42,
            1,
            "Installing the operating system",
            &notes,
            &copy::writing(Some(Path::new("/var/log/tect-installer.log"))),
        ),
    );
}

#[test]
fn the_leave_and_completion_screens_use_production_actions() {
    let leave = leave_options();
    assert_screen(
        "leave",
        screen().picker(copy::LEAVING, &leave, None, copy::INSTALL_KEYS, 2),
    );

    let offered = Offered::Unavailable(copy::AUTO_NO_POLICY.to_string());
    let mut completion = done_rows(
        Some("0123-4567-89ab-cdef"),
        Some(Path::new("/var/log/tect-installer.log")),
        Some(copy::shim_steps()),
        &[],
        &["/dev/vda3".to_string()],
        &offered,
    );
    completion.extend(done_actions(&offered));
    let rendered = assert_screen(
        "completion",
        screen().picker(
            copy::INSTALL_DONE,
            &completion,
            None,
            copy::DONE_KEYS,
            completion.len() - 1,
        ),
    );
    assert_style_at(
        &rendered,
        4,
        33,
        "modifier: DIM",
        "the unavailable automatic finalizer",
    );
}
