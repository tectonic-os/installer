use super::*;

/// Names the backend command. The live environment puts fisherman on `PATH`,
/// so a headless run elsewhere can shadow it.
pub(crate) const BACKEND: &str = "fisherman";

/// Bounds how long the screen waits for a fisherman line before it redraws.
/// The spinner must keep turning. A shorter wait repaints a quiet install
/// eight times a second.
pub(crate) const TICK: std::time::Duration = std::time::Duration::from_millis(120);

/// Holds one line of fisherman's event stream. The gauge wants how far the
/// run has got and the log wants what the line said, so the installer reads
/// each line once and renders it where it is needed.
pub(crate) enum Event {
    /// Carries the work finished before this step, the share this step adds,
    /// and what is happening. A bar needs `weight_pct` to draw the span the
    /// step is inside.
    Step(u16, u16, String),
    /// Adds a line under the step.
    Note(String),
    /// Ends the run at the full bar.
    Done(String),
    /// Carries the only copy of the recovery key there will ever be. If the
    /// TPM stops answering, this key is what opens the disk. It must never
    /// reach the log, because a key written to removable media makes the
    /// stick open the disk.
    Recovery(String),
    /// Holds a line fisherman's own backends wrote. The installer keeps each
    /// line as it came, because a failed install is read back from them.
    Other(String),
}

impl Event {
    pub(crate) fn of(line: &str) -> Self {
        let Ok(event) = Json::parse(line) else {
            return Self::Other(line.to_string());
        };
        let text = |key: &str| json::text(&event, key).unwrap_or_default();
        let count = |key: &str| json::number(&event, key).unwrap_or(0);
        match json::text(&event, "type").as_deref() {
            Some("step") => Self::Step(
                count("cumulative_pct") as u16,
                count("weight_pct") as u16,
                format!(
                    "{}/{} {}",
                    count("step"),
                    count("total_steps"),
                    text("step_name")
                ),
            ),
            Some("info" | "substep") => Self::Note(text("message")),
            Some("complete") => Self::Done(text("message")),
            Some("recovery_key") => Self::Recovery(text("key")),
            _ => Self::Other(line.to_string()),
        }
    }

    /// Holds back the recovery key and returns every other line.
    pub(crate) fn logged(&self) -> Option<String> {
        match self {
            Self::Recovery(_) => None,
            said => Some(said.say()),
        }
    }

    /// Renders the line a run with no screen prints.
    pub(crate) fn say(&self) -> String {
        match self {
            Self::Step(pct, _, what) => format!("[{pct:>3}%] {what}"),
            Self::Note(message) => format!("       {message}"),
            Self::Done(message) => format!("[100%] {message}"),
            Self::Recovery(key) => format!("\n{}\n", copy::recovery(key)),
            Self::Other(line) => line.clone(),
        }
    }
}
