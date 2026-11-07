//! Desktop notifications with an optional click action (T13/T27).
//!
//! Uses `notify-send` (libnotify), started directly without a shell. With an action, the process
//! stays alive until the notification is closed and prints the name of the action the user chose,
//! which is how a click opens the right title in the app.
//!
//! Limits, stated plainly: clicking only works if the notification server supports actions (most
//! full desktops do; some minimal ones do not and then simply show the notification). Alerts are
//! only shown while the app is running, as with the checks that produce them.
use anyhow::{bail, Context, Result};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};

const MAX_TITLE_CHARS: usize = 200;
const MAX_BODY_CHARS: usize = 500;

#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub icon: String,
    /// `(name, label)` of a single action, for example `("open", "Open")`.
    pub action: Option<(String, String)>,
}

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// How long to wait for the notifier to fail before treating the notification as shown.
    pub settle: Duration,
    /// How long to wait for a click before giving up and ending the notifier.
    pub click_window: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { settle: Duration::from_millis(1500), click_window: Duration::from_secs(30 * 60) }
    }
}

fn clean(text: &str, limit: usize) -> String {
    text.chars().filter(|c| !c.is_control() || *c == ' ').take(limit).collect::<String>().trim().to_string()
}

/// Show a notification. Returns once it is known to have been accepted (or has failed). If an
/// action was requested and is chosen later, `on_action` runs with the chosen action's name.
pub async fn send(program: &str, notification: &Notification, timing: Timing, on_action: impl FnOnce(String) + Send + 'static) -> Result<()> {
    let title = clean(&notification.title, MAX_TITLE_CHARS);
    if title.is_empty() {
        bail!("A notification needs a title");
    }
    let mut command = tokio::process::Command::new(program);
    command.arg("--app-name=Cloudstream Desktop");
    if !notification.icon.is_empty() {
        command.arg(format!("--icon={}", clean(&notification.icon, 100)));
    }
    let allowed_action = notification.action.as_ref().map(|(name, _)| name.clone());
    if let Some((name, label)) = &notification.action {
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            bail!("Invalid notification action");
        }
        command.arg(format!("--action={name}={}", clean(label, 40)));
    }
    // `--` ends option parsing: a title that starts with a dash is still just text.
    command.arg("--").arg(&title).arg(clean(&notification.body, MAX_BODY_CHARS));
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    let mut child = command.spawn().context("Desktop notifications are unavailable")?;
    let stdout = child.stdout.take().context("Desktop notifications are unavailable")?;

    match tokio::time::timeout(timing.settle, child.wait()).await {
        Ok(Ok(status)) if status.success() => {
            // It finished at once: shown (or an action was already chosen and was printed).
            let mut line = String::new();
            let _ = tokio::time::timeout(Duration::from_millis(200), BufReader::new(stdout).read_line(&mut line)).await;
            deliver(&line, allowed_action.as_deref(), on_action);
            Ok(())
        }
        Ok(_) => bail!("Desktop notifications are unavailable"),
        Err(_) => {
            // Still running after the settle time: it was accepted and is waiting for the user.
            if allowed_action.is_some() {
                tokio::spawn(async move {
                    let mut line = String::new();
                    let read = tokio::time::timeout(timing.click_window, BufReader::new(stdout).read_line(&mut line)).await;
                    if read.is_ok() {
                        deliver(&line, allowed_action.as_deref(), on_action);
                    }
                    let _ = child.kill().await;
                });
            } else {
                tokio::spawn(async move {
                    let _ = tokio::time::timeout(timing.click_window, child.wait()).await;
                    let _ = child.kill().await;
                });
            }
            Ok(())
        }
    }
}

/// Only the one action we offered is acted on; anything else the server prints is ignored.
fn deliver(line: &str, allowed: Option<&str>, on_action: impl FnOnce(String)) {
    let chosen = line.trim();
    if !chosen.is_empty() && Some(chosen) == allowed {
        on_action(chosen.to_string());
    }
}
