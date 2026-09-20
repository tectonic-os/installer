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
