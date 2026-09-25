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
