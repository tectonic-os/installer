use super::*;

/// This run has nowhere writable, so the last row says the screen is the
/// only copy. The user then reads the key and the next steps off the panel,
/// which is why every row keeps its place and its width.
#[test]
fn the_last_screen_heads_the_key_and_the_steps() {
    let volumes = ["encrypted disk: /dev/vda".to_string()];
    let rows = done_rows(
        Some("cafebabe"),
        None,
        Some(copy::next_steps_setup()),
        &[],
        &volumes,
        &Offered::None,
    );
    let said: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(said[0], copy::RECOVERY_HEADING);
    assert_eq!(said[1], copy::write_down());
    assert_eq!(said[2], "");
    assert_eq!(said[3], "encrypted disk: /dev/vda");
    assert_eq!(said[4], "");
    assert_eq!(said[5], "cafebabe");
    assert_eq!(said[6], "");
    assert_eq!(said[7], copy::NEXT_STEPS_HEADING);
    assert_eq!(said[said.len() - 1], copy::logging(None));
    assert!(
        rows[0].heading && rows[7].heading,
        "the headings are headings"
    );
    assert!(
        !rows[1].heading && !rows[3].heading && !rows[5].heading,
        "the content is content"
    );
    let wrapped: Vec<&str> = said[8..said.len() - 1].to_vec();
    assert_eq!(wrapped.join(" "), copy::next_steps_setup());
    assert!(
        wrapped
            .iter()
            .all(|line| line.chars().count() <= common::ui::ROW_ROOM),
        "{wrapped:?}"
    );
}

/// Holds a finalize the completion loop can take, with the key the user typed
/// kept out of every file.
fn a_ready() -> Ready {
    Ready {
        image: "example.invalid/image:1".to_string(),
        disk: "/dev/vda".to_string(),
        partition: "/dev/vda2".to_string(),
        key: Key::Passphrase("recovery".to_string()),
        pin: false,
    }
}

/// A failed finalize must leave the last screen up with the reason. The
/// recovery key that screen shows is in no file, so a restart past it would
/// leave the user with a disk they cannot open and a key they never read.
#[test]
fn a_failed_finalize_draws_the_screen_again_with_the_reason() {
    let mut offered = Offered::Ready(a_ready());
    let mut drawn = Vec::new();
    let mut restarts = 0;
    choosing(
        &mut offered,
        |offered| {
            drawn.push(match offered {
                Offered::Ready(_) => 0,
                Offered::Unavailable(_) => 1,
                Offered::None => 2,
            });
            Ok(Some(drawn.len() - 1))
        },
        |_| Err("the seal failed".to_string()),
        || {
            restarts += 1;
            Ok(())
        },
    )
    .expect("the manual action ends the loop");
    assert_eq!(drawn, [0, 1], "the screen is drawn again after the failure");
    assert_eq!(restarts, 1);
    match &offered {
        Offered::Unavailable(why) => assert_eq!(why, copy::AUTO_FAILED),
        _ => panic!("a failed finalize must leave the automatic action unavailable"),
    }
}

/// A successful finalize restarts at once, so no action can be taken between
/// the credential write and the boot that reads it.
#[test]
fn a_successful_finalize_restarts_at_once() {
    let mut offered = Offered::Ready(a_ready());
    let mut drawn = 0;
    let mut finalized = 0;
    let mut restarts = 0;
    choosing(
        &mut offered,
        |_| {
            drawn += 1;
            Ok(Some(0))
        },
        |_| {
            finalized += 1;
            Ok(())
        },
        || {
            restarts += 1;
            Ok(())
        },
    )
    .expect("the restart ends the loop");
    assert_eq!((drawn, finalized, restarts), (1, 1, 1));
}

/// Esc on the last screen ends the run without a finalize and without a
/// restart, so the user keeps the machine as the install left it.
#[test]
fn esc_ends_the_last_screen_without_restarting() {
    let mut offered = Offered::Ready(a_ready());
    choosing(
        &mut offered,
        |_| Ok(None),
        |_| panic!("no action was taken"),
        || panic!("nothing asked for a restart"),
    )
    .expect("esc is an ending");
    assert!(matches!(offered, Offered::Ready(_)));
}

/// A staged enrolment draws what each action does directly above the actions,
/// wrapped to the rows, and the automatic action's window draws in the
/// warning colour. The manual restart alone draws neither.
#[test]
fn the_last_screen_explains_the_two_actions() {
    let ready = |pin| {
        Offered::Ready(Ready {
            image: "example.invalid/image:1".to_string(),
            disk: "/dev/vda".to_string(),
            partition: "/dev/vda2".to_string(),
            key: Key::Passphrase("recovery".to_string()),
            pin,
        })
    };
    let rows = done_rows(None, None, None, &[], &[], &ready(true));
    let said: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
    let window = said.len() - 1;
    let explained = &said[..window];
    let start = explained
        .iter()
        .position(|line| line.starts_with("The installer will reboot"))
        .expect("the explanation is drawn");
    let paragraphs = copy::automatic_explanation();
    let joined = explained[start..]
        .iter()
        .filter(|line| !line.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        joined,
        paragraphs.join(" "),
        "every wrapped line is part of the one explanation"
    );
    assert!(
        explained[start..]
            .iter()
            .all(|line| line.chars().count() <= common::ui::ROW_ROOM),
        "{explained:?}"
    );
    let second = explained
        .iter()
        .position(|line| line.starts_with("If you would prefer"))
        .expect("the manual paragraph is drawn");
    assert!(explained[second - 1].is_empty(), "a blank parts them");
    assert_eq!(said[window], copy::AUTO_WINDOW_PIN);
    assert!(rows[window].warning, "the window is the must-read row");
    let plain = done_rows(None, None, None, &[], &[], &ready(false));
    assert_eq!(
        plain.last().map(|row| row.label.as_str()),
        Some(copy::AUTO_WINDOW)
    );
    let refused = done_rows(
        None,
        None,
        None,
        &[],
        &[],
        &Offered::Unavailable(copy::AUTO_NO_POLICY.to_string()),
    );
    assert!(
        refused.last().is_some_and(|row| !row.warning),
        "an unavailable action draws no window"
    );
}
