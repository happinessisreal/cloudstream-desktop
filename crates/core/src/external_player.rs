//! Opening a stream in another media player (T27).
//!
//! Cloudstream Android lets the user pick an external player app. On Linux that means a program
//! such as VLC, or any command the user configures. The rules, all enforced here:
//!
//! * The program and its arguments are built as an argument *vector* and started directly. There
//!   is no shell, so quotes, `;`, `$()` or backticks in a title, URL or extension-supplied value
//!   are just characters.
//! * A custom command is a template where each argument is one item; `{url}`, `{title}`,
//!   `{referer}`, `{user_agent}`, `{start}` and `{subtitle}` are substituted *inside* that item,
//!   never split into several.
//! * Only what the player can carry is passed. VLC takes a referrer and a user agent from the
//!   command line, not arbitrary headers; if a stream needs other headers the launch is refused
//!   with the names, instead of starting a player that will fail on its own.
//! * Streams that depend on the app's provider session (`resourceHandle`) are not handed out:
//!   their address only works through the built-in player.
use crate::models::{Stream, Subtitle};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, path::{Path, PathBuf}};

pub const MAX_TEMPLATE_ARGUMENTS: usize = 32;
const MAX_ARGUMENT_CHARS: usize = 1024;
const PLACEHOLDERS: [&str; 6] = ["{url}", "{title}", "{referer}", "{user_agent}", "{start}", "{subtitle}"];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExternalPlayerKind {
    /// Use only the built-in player.
    #[default]
    None,
    Vlc,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalPlayerSettings {
    #[serde(default)]
    pub kind: ExternalPlayerKind,
    /// A program name found on `PATH`, or an absolute path. Defaults to `vlc` for VLC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// Custom command only: one argument template per item.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for ExternalPlayerSettings {
    fn default() -> Self {
        Self { kind: ExternalPlayerKind::None, program: None, args: Vec::new(), extra: BTreeMap::new() }
    }
}

impl ExternalPlayerSettings {
    pub fn validate(&self) -> Result<()> {
        if let Some(program) = &self.program {
            if program.trim().is_empty() || program.contains('\0') || program.chars().any(char::is_control) {
                bail!("Enter the name or full path of a program");
            }
            if program.contains('/') && !Path::new(program).is_absolute() {
                bail!("Use a program name from your PATH or a full path");
            }
        }
        if self.kind == ExternalPlayerKind::Custom {
            if self.program.is_none() {
                bail!("Choose the program to run for your custom player");
            }
            if self.args.is_empty() || !self.args.iter().any(|a| a.contains("{url}")) {
                bail!("A custom player needs at least one argument containing {{url}}");
            }
        }
        if self.args.len() > MAX_TEMPLATE_ARGUMENTS {
            bail!("A custom player can have at most {MAX_TEMPLATE_ARGUMENTS} arguments");
        }
        for arg in &self.args {
            if arg.chars().count() > MAX_ARGUMENT_CHARS || arg.contains('\0') || arg.chars().any(|c| c.is_control()) {
                bail!("A custom player argument is too long or contains control characters");
            }
            // Only the known placeholders may appear, so a typo is caught here, not at launch.
            let mut rest = arg.as_str();
            while let Some(start) = rest.find('{') {
                let Some(end) = rest[start..].find('}') else { break };
                let token = &rest[start..=start + end];
                if !PLACEHOLDERS.contains(&token) && token.len() > 2 && token[1..token.len() - 1].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                    bail!("Unknown placeholder {token}. Use {}", PLACEHOLDERS.join(", "));
                }
                rest = &rest[start + end + 1..];
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DetectedPlayer {
    pub kind: ExternalPlayerKind,
    pub name: String,
    pub program: String,
}

fn find_in_path(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let path = PathBuf::from(program);
        return is_executable(&path).then_some(path);
    }
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .filter(|dir| !dir.is_empty() && Path::new(dir).is_absolute())
        .map(|dir| Path::new(dir).join(program))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// Players found on this machine that the app knows how to drive.
pub fn detect_players() -> Vec<DetectedPlayer> {
    [("vlc", "VLC", ExternalPlayerKind::Vlc)]
        .into_iter()
        .filter_map(|(program, name, kind)| find_in_path(program).map(|path| DetectedPlayer { kind, name: name.into(), program: path.display().to_string() }))
        .collect()
}

pub struct LaunchRequest<'a> {
    pub stream: &'a Stream,
    pub title: &'a str,
    pub start_seconds: f64,
    pub subtitles: &'a [Subtitle],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Things that were left out or could not be carried over, for the user.
    pub warnings: Vec<String>,
}

fn header<'a>(stream: &'a Stream, wanted: &str) -> Option<&'a str> {
    stream.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(wanted)).map(|(_, v)| v.as_str())
}

fn clean(value: &str) -> Result<&str> {
    if value.chars().any(|c| c == '\0' || c == '\r' || c == '\n') {
        bail!("A value contains line breaks, which players cannot take on a command line");
    }
    Ok(value)
}

/// Build the program and argument vector for a launch. Pure apart from looking up the program.
pub fn plan_launch(settings: &ExternalPlayerSettings, request: &LaunchRequest) -> Result<LaunchPlan> {
    settings.validate()?;
    if settings.kind == ExternalPlayerKind::None {
        bail!("No external player is chosen. Pick one in Settings");
    }
    let stream = request.stream;
    if stream.resource_handle.is_some() {
        bail!("This source protects its links, so it only plays in the built-in player");
    }
    crate::repository::network_url(&stream.url)?;
    if crate::streams::is_playlist(stream) {
        bail!("Multi-segment playlists only play in the built-in player");
    }
    if !stream.audio_tracks.is_empty() {
        bail!("This stream has separate audio that an external player cannot combine. Use the built-in player");
    }
    if !request.start_seconds.is_finite() || request.start_seconds < 0.0 {
        bail!("Invalid start position");
    }
    let referer = if !stream.referer.is_empty() { Some(stream.referer.as_str()) } else { header(stream, "referer") };
    let user_agent = header(stream, "user-agent");
    let mut warnings = Vec::new();
    let (public_subtitles, protected): (Vec<&Subtitle>, Vec<&Subtitle>) = request.subtitles.iter().partition(|s| s.headers.is_empty());
    if !protected.is_empty() {
        warnings.push(format!("{} subtitle track(s) need a protected download and were left out", protected.len()));
    }
    for sub in &public_subtitles {
        crate::repository::network_url(&sub.url)?;
    }
    let start = request.start_seconds.floor() as u64;

    match settings.kind {
        ExternalPlayerKind::Vlc => {
            let other: Vec<&String> = stream.headers.keys().filter(|k| !k.eq_ignore_ascii_case("referer") && !k.eq_ignore_ascii_case("user-agent")).collect();
            if !other.is_empty() {
                bail!(
                    "VLC cannot send these request headers: {}. Use the built-in player for this source",
                    other.iter().map(|h| h.as_str()).collect::<Vec<_>>().join(", ")
                );
            }
            let program = find_in_path(settings.program.as_deref().unwrap_or("vlc")).context("VLC was not found. Install it or choose another player")?;
            let mut args = Vec::new();
            if start > 0 {
                args.push(format!("--start-time={start}"));
            }
            if let Some(referer) = referer {
                args.push(format!("--http-referrer={}", clean(referer)?));
            }
            if let Some(agent) = user_agent {
                args.push(format!("--http-user-agent={}", clean(agent)?));
            }
            for sub in public_subtitles {
                args.push(format!("--sub-file={}", clean(&sub.url)?));
            }
            // `--` ends option parsing, so an address can never be read as a flag.
            args.push("--".into());
            args.push(stream.url.clone());
            Ok(LaunchPlan { program, args, warnings })
        }
        ExternalPlayerKind::Custom => {
            let program = find_in_path(settings.program.as_deref().unwrap_or_default()).with_context(|| format!("The program '{}' was not found or is not executable", settings.program.as_deref().unwrap_or_default()))?;
            let subtitle = public_subtitles.first().map(|s| s.url.as_str());
            let templates_text = settings.args.join("\n");
            let carried = |placeholder: &str| templates_text.contains(placeholder);
            let ignored: Vec<&String> = stream.headers.keys().filter(|k| {
                (k.eq_ignore_ascii_case("referer") && !carried("{referer}")) || (k.eq_ignore_ascii_case("user-agent") && !carried("{user_agent}")) || !(k.eq_ignore_ascii_case("referer") || k.eq_ignore_ascii_case("user-agent"))
            }).collect();
            if !ignored.is_empty() {
                warnings.push(format!("These request headers are not passed to your player: {}", ignored.iter().map(|h| h.as_str()).collect::<Vec<_>>().join(", ")));
            }
            let values: [(&str, Option<String>); 6] = [
                ("{url}", Some(stream.url.clone())),
                ("{title}", Some(request.title.to_string())),
                ("{referer}", referer.map(str::to_string)),
                ("{user_agent}", user_agent.map(str::to_string)),
                ("{start}", Some(start.to_string())),
                ("{subtitle}", subtitle.map(str::to_string)),
            ];
            let mut args = Vec::new();
            for template in &settings.args {
                // An argument that is only a placeholder with no value is dropped entirely.
                if let Some((_, None)) = values.iter().find(|(name, _)| template.trim() == *name) {
                    continue;
                }
                let mut out = template.clone();
                for (name, value) in &values {
                    if out.contains(name) {
                        out = out.replace(name, clean(value.as_deref().unwrap_or(""))?);
                    }
                }
                args.push(out);
            }
            Ok(LaunchPlan { program, args, warnings })
        }
        ExternalPlayerKind::None => unreachable!(),
    }
}

/// Start the player detached from the app, without a shell and without inheriting its streams.
pub fn launch(plan: &LaunchPlan) -> Result<u32> {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(&plan.program);
    command.args(&plan.args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // Its own process group: closing this app does not close the player, and signals do not leak.
    command.process_group(0);
    let mut child = command.spawn().with_context(|| format!("Could not start {}", plan.program.display()))?;
    let pid = child.id();
    // Reap it when it exits so it never lingers as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}
