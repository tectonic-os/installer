use super::*;

/// `Event::of` reads each fisherman line once. The recovery key is the one
/// event the user cannot ask fisherman for again, so `say` prints it whole.
#[test]
fn every_event_reads_as_a_line_and_anything_else_passes_through() {
    let said = |line: &str| Event::of(line).say();
    assert_eq!(
        said(
            r#"{"type":"step","step":7,"total_steps":12,"step_name":"install OS","cumulative_pct":9,"weight_pct":87,"elapsed_ms":4210}"#
        ),
        "[  9%] 7/12 install OS"
    );
    assert_eq!(
        said(r#"{"type":"info","message":"Live environment detected"}"#),
        "       Live environment detected"
    );
    assert_eq!(
        said(r#"{"type":"complete","message":"Installation complete"}"#),
        "[100%] Installation complete"
    );
    let key = said(r#"{"type":"recovery_key","key":"abcd-efgh"}"#);
    assert!(
        key.contains("abcd-efgh") && key.contains(copy::write_down()),
        "{key}"
    );
    // A failed install is read back from fisherman's own backend lines.
    assert_eq!(said("bootc: pulling layer 3/9"), "bootc: pulling layer 3/9");
}

/// The install log is written beside the payload on the installer media, so
/// a recovery key in the log would make that stick open the disk.
#[test]
fn the_log_holds_every_event_but_the_key() {
    let logged = |line: &str| Event::of(line).logged();
    assert!(logged(r#"{"type":"recovery_key","key":"abcd-efgh"}"#).is_none());
    assert!(logged(r#"{"type":"info","message":"Live environment"}"#).is_some());
    assert!(logged("bootc: pulling layer 3/9").is_some());
}
