use crate::models::*;
use crate::streams;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::{Child, Command},
    sync::{broadcast, mpsc, oneshot, Mutex, OnceCell, RwLock},
};

/// A provider subtitle and optional cleanup to perform after playback starts.
#[derive(Clone)]
pub struct PreparedSubtitle {
    pub subtitle: Subtitle,
    pub cleanup: Option<(String, bool, crate::subtitles::TextTransforms)>,
}

/// Properties mpv is asked to push to us. Ids are `1..=len`; command replies use ids from
/// `COMMAND_ID_BASE`, so the two never collide.
pub const OBSERVED_PROPERTIES: [(u64, &str); 15] = [
    (1, "time-pos"),
    (2, "duration"),
    (3, "pause"),
    (4, "volume"),
    (5, "speed"),
    (6, "track-list"),
    (7, "mute"),
    (8, "fullscreen"),
    (9, "audio-delay"),
    (10, "video-rotate"),
    (11, "panscan"),
    (12, "keepaspect"),
    (13, "video-aspect-override"),
    (14, "hwdec-current"),
    (15, "sub-delay"),
];
const COMMAND_ID_BASE: u64 = 100;

/// Dynamic range compression used for the optional audio compressor. ffmpeg's `acompressor`
/// threshold is linear amplitude (0.089 ≈ -21 dBFS).
pub const COMPRESSOR_FILTER: &str = "acompressor=threshold=0.089:ratio=4:attack=20:release=250:makeup=2";

pub struct Player {
    directory: PathBuf,
    fonts_directory: Option<PathBuf>,
    session: Arc<Mutex<Option<ActiveSession>>>,
    state: Arc<RwLock<PlayerState>>,
    events_tx: broadcast::Sender<PlayerEvent>,
    current_watch: Arc<Mutex<Option<(WatchEntry, String)>>>,
    req_counter: Arc<AtomicU64>,
    subtitle_selection_epoch: Arc<AtomicU64>,
    queue: Arc<RwLock<Option<PlaybackQueue>>>,
    policy: Arc<RwLock<PlaybackPolicy>>,
    prefs: Arc<RwLock<PlayerPreferences>>,
    tracks: Arc<RwLock<TrackChoice>>,
    pending_stream: Mutex<Option<ActiveStream>>,
    speed_hold: Mutex<Option<(String, f64)>>,
    capabilities: OnceCell<PlayerCapabilities>,
    /// Extra mpv arguments appended after the defaults (for example `--vo=null` for headless runs).
    launch_args: Vec<String>,
}

struct ActiveSession {
    session_id: String,
    #[allow(dead_code)]
    profile_id: String,
    child: Option<Child>,
    socket: PathBuf,
    command_tx: mpsc::Sender<(Value, oneshot::Sender<Result<Value>>)>,
    subtitle_assets: Arc<Mutex<Vec<PathBuf>>>,
}

/// Map the preview/settings model to mpv's text-subtitle options. mpv shares one color between
/// shadow and background boxes; in box mode its shadow offset controls the box padding.
pub fn subtitle_style_properties(style: &SubtitleStyle) -> Vec<(&'static str, Value)> {
    let alpha = ((u16::from(style.background_opacity) * 255 + 50) / 100) as u8;
    let background = format!("#{alpha:02X}{}", style.background_color.strip_prefix('#').unwrap_or("000000"));
    vec![
        ("sub-font", json!(style.font)), ("sub-font-size", json!(style.size)),
        ("sub-color", json!(style.color)), ("sub-bold", json!(style.bold)), ("sub-italic", json!(style.italic)),
        ("sub-outline-color", json!(style.outline_color)), ("sub-outline-size", json!(style.outline_size)),
        ("sub-shadow-offset", json!(style.shadow_offset)), ("sub-back-color", json!(background)),
        ("sub-border-style", json!(if style.background_box { "background-box" } else { "outline-and-shadow" })),
        ("sub-align-x", json!(style.align_x)), ("sub-align-y", json!(style.align_y)),
        ("sub-margin-y", json!(style.vertical_margin)), ("sub-ass-override", json!(if style.override_ass { "force" } else if *style == SubtitleStyle::default() { "scale" } else { "no" })),
    ]
}

/// Build the mpv command line that realises the user's preferences. Options the installed mpv
/// does not support are skipped and reported back as human-readable notes instead of making mpv
/// refuse to start.
pub fn launch_options(
    prefs: &PlayerPreferences,
    policy: &PlaybackPolicy,
    carry: &TrackChoice,
    caps: &PlayerCapabilities,
) -> (Vec<String>, Vec<String>) {
    let mut args = Vec::new();
    let mut notes = Vec::new();
    let gate = |id: &str, label: &str, notes: &mut Vec<String>| -> bool {
        if caps.supports(id) {
            true
        } else {
            let why = caps.reason(id).unwrap_or_else(|| "not supported by this mpv".into());
            notes.push(format!("{label} was skipped: {why}"));
            false
        }
    };

    if gate("volume", "Volume", &mut notes) {
        args.push(format!("--volume={}", prefs.volume));
    }
    if prefs.muted && gate("mute", "Mute", &mut notes) {
        args.push("--mute=yes".into());
    }
    if (prefs.speed - 1.0).abs() > f64::EPSILON && gate("speed", "Playback speed", &mut notes) {
        args.push(format!("--speed={}", prefs.speed));
    }
    if prefs.fullscreen && gate("fullscreen", "Fullscreen", &mut notes) {
        args.push("--fullscreen=yes".into());
    }
    if prefs.aspect != AspectMode::Fit && gate("aspect", "Aspect ratio", &mut notes) {
        if let Some((keep, pan, aspect)) = prefs.aspect.mpv_values() {
            args.push(format!("--keepaspect={}", if keep { "yes" } else { "no" }));
            args.push(format!("--panscan={pan}"));
            let aspect = match aspect {
                Value::String(ratio) => ratio,
                other => other.to_string(),
            };
            args.push(format!("--video-aspect-override={aspect}"));
        }
    }
    if prefs.audio_delay != 0.0 && gate("audio_delay", "Audio delay", &mut notes) {
        args.push(format!("--audio-delay={}", prefs.audio_delay));
    }
    if prefs.subtitle_delay != 0.0 && gate("subtitle_delay", "Subtitle delay", &mut notes) {
        args.push(format!("--sub-delay={}", prefs.subtitle_delay));
    }
    if prefs.subtitle_style != SubtitleStyle::default() && gate("subtitle_style", "Subtitle appearance", &mut notes) {
        for (name, value) in subtitle_style_properties(&prefs.subtitle_style) {
            let text = match &value {
                Value::String(text) => text.clone(),
                Value::Bool(value) => if *value { "yes".into() } else { "no".into() },
                _ => value.to_string(),
            };
            args.push(format!("--{name}={text}"));
        }
    }
    if gate("hardware_decoding", "Decoding preference", &mut notes) {
        args.push(format!("--hwdec={}", prefs.decoding.mpv_value()));
    }
    if prefs.buffer_ram_mb > 0 && gate("buffer_size", "Buffer size", &mut notes) {
        args.push(format!("--demuxer-max-bytes={}MiB", prefs.buffer_ram_mb));
    }
    if prefs.buffer_seconds > 0 && gate("buffer_time", "Buffer length", &mut notes) {
        args.push(format!("--demuxer-readahead-secs={}", prefs.buffer_seconds));
    }
    if prefs.audio_compressor && gate("audio_compressor", "Audio compressor", &mut notes) {
        args.push(format!("--af={COMPRESSOR_FILTER}"));
    }
    if policy.start_paused {
        args.push("--pause=yes".into());
    }
    let choice = effective_track_choice(prefs, carry);
    if choice.audio_enabled == Some(false) {
        args.push("--aid=no".into());
    } else if let Some(language) = choice.audio_language.as_deref().filter(|l| safe_language(l)) {
        args.push(format!("--alang={language}"));
    }
    if choice.subtitles_enabled == Some(false) {
        args.push("--sid=no".into());
    } else if let Some(language) = choice.subtitle_language.as_deref().filter(|l| safe_language(l)) {
        args.push(format!("--slang={language}"));
    }
    (args, notes)
}

fn effective_track_choice(prefs: &PlayerPreferences, carry: &TrackChoice) -> TrackChoice {
    TrackChoice {
        audio_enabled: carry.audio_enabled.or(prefs.audio_enabled),
        audio_language: carry.audio_language.clone().or_else(|| prefs.audio_language.clone()),
        subtitles_enabled: carry.subtitles_enabled.or(prefs.subtitles_enabled),
        subtitle_language: carry.subtitle_language.clone().or_else(|| prefs.subtitle_language.clone()),
    }
}

/// Language tags come from mpv's track list; only pass plain tags back on a command line.
fn safe_language(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= 12 && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

async fn run_mpv(args: &[&str]) -> Option<String> {
    let mut command = Command::new("mpv");
    command
        .arg("--no-config")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(8), command.output())
        .await
        .ok()?
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Ask the installed mpv what it supports. Capabilities come from `--list-options`,
/// `--list-protocols` and `--af=help`, not from the version number alone.
pub async fn probe_capabilities() -> PlayerCapabilities {
    let (version, options, protocols, filters) = tokio::join!(
        run_mpv(&["--version"]),
        run_mpv(&["--list-options"]),
        run_mpv(&["--list-protocols"]),
        run_mpv(&["--af=help"]),
    );
    let Some(version_text) = version else {
        let reason = "mpv is not installed or did not respond. Install mpv to enable playback.";
        return PlayerCapabilities {
            available: false,
            version: None,
            capabilities: CAPABILITY_CATALOG
                .iter()
                .map(|(id, label, _)| CapabilityStatus {
                    id: (*id).into(),
                    label: (*label).into(),
                    supported: false,
                    reason: Some(reason.into()),
                })
                .collect(),
        };
    };
    let version = version_text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .map(|v| v.trim_start_matches('v').to_string());
    let option_names: HashSet<String> = options
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let name = line.strip_prefix("--")?.split_whitespace().next()?;
            Some(name.trim_end_matches('=').to_string())
        })
        .collect();
    let have_options = !option_names.is_empty();
    let has = |names: &[&str]| have_options && names.iter().all(|n| option_names.contains(*n));
    let has_edl = protocols.is_some_and(|p| p.contains("edl://"));
    let has_compressor = filters.is_some_and(|f| f.contains("acompressor"));

    let capabilities = CAPABILITY_CATALOG
        .iter()
        .map(|(id, label, needs)| {
            let (supported, reason) = match *id {
                "video_delay" => (
                    false,
                    Some("mpv has no independent video delay. Use the audio delay to line up sound and picture.".to_string()),
                ),
                "buffer_disk" => (
                    false,
                    Some("A size-limited disk cache is not exposed by mpv. Use the memory buffer instead.".to_string()),
                ),
                "audio_compressor" => {
                    if has(needs) && has_compressor {
                        (true, None)
                    } else {
                        (false, Some("This mpv build has no acompressor audio filter.".to_string()))
                    }
                }
                "playlist_timeline" => {
                    if has_edl {
                        (true, None)
                    } else {
                        (false, Some("This mpv build cannot play edl:// timelines, so multi-segment playlists cannot be played.".to_string()))
                    }
                }
                _ => {
                    if has(needs) {
                        (true, None)
                    } else {
                        (
                            false,
                            Some(format!(
                                "This mpv ({}) does not provide the {} option.",
                                version.as_deref().unwrap_or("unknown version"),
                                needs.first().copied().unwrap_or(id)
                            )),
                        )
                    }
                }
            };
            CapabilityStatus { id: (*id).into(), label: (*label).into(), supported, reason }
        })
        .collect();
    PlayerCapabilities { available: true, version, capabilities }
}

/// `(id, label, mpv options that must exist)`.
const CAPABILITY_CATALOG: [(&str, &str, &[&str]); 18] = [
    ("volume", "Volume", &["volume"]),
    ("mute", "Mute", &["mute"]),
    ("seek", "Seek", &["start"]),
    ("speed", "Playback speed", &["speed"]),
    ("fullscreen", "Fullscreen", &["fullscreen"]),
    ("aspect", "Aspect ratio", &["keepaspect", "panscan", "video-aspect-override"]),
    ("audio_delay", "Audio delay", &["audio-delay"]),
    ("subtitle_delay", "Subtitle delay", &["sub-delay"]),
    ("subtitle_custom_font", "Custom subtitle font", &["sub-fonts-dir"]),
    ("subtitle_style", "Subtitle appearance", &["sub-font", "sub-font-size", "sub-color", "sub-bold", "sub-italic", "sub-outline-color", "sub-outline-size", "sub-shadow-offset", "sub-back-color", "sub-border-style", "sub-align-x", "sub-align-y", "sub-margin-y", "sub-ass-override"]),
    ("video_delay", "Video delay", &[]),
    ("rotation", "Rotation", &["video-rotate"]),
    ("hardware_decoding", "Hardware decoding", &["hwdec"]),
    ("buffer_size", "Buffer size", &["demuxer-max-bytes"]),
    ("buffer_time", "Buffer length", &["demuxer-readahead-secs"]),
    ("buffer_disk", "Disk buffer", &[]),
    ("audio_compressor", "Audio compressor", &["af"]),
    ("playlist_timeline", "Multi-segment playlists", &[]),
];

/// Turn mpv's terse IPC errors into something a person can act on.
fn friendly_error(command: &PlayerCommand, raw: &str) -> anyhow::Error {
    let action = command.describe();
    let detail = raw.trim_start_matches("Player error: ");
    match detail {
        "property unavailable" | "error running command" => {
            anyhow::anyhow!("Could not {action}: the video has not finished loading yet")
        }
        "property not found" => anyhow::anyhow!("Could not {action}: this mpv does not support it"),
        "unsupported format for accessing property" | "invalid parameter" => {
            anyhow::anyhow!("Could not {action}: mpv rejected that value")
        }
        other => anyhow::anyhow!("Could not {action}: {other}"),
    }
}

fn player_tracks_from(list: &Value) -> Vec<PlayerTrack> {
    list.as_array().into_iter().flatten().filter_map(|track| {
        let kind = track.get("type")?.as_str()?;
        if kind != "audio" && kind != "sub" { return None; }
        let flag = |key: &str| track.get(key).and_then(Value::as_bool).unwrap_or(false);
        let text = |key: &str| track.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(PlayerTrack { id: track.get("id")?.as_i64()?, kind: kind.into(), selected: flag("selected"),
            external: flag("external"), forced: flag("forced"), hearing_impaired: flag("hearing-impaired"),
            language: text("lang"), title: text("title"), codec: text("codec") })
    }).collect()
}

fn track_choice_from(list: &Value) -> TrackChoice {
    let mut choice = TrackChoice::default();
    let Some(tracks) = list.as_array() else {
        return choice;
    };
    let mut any_subtitle = false;
    let mut any_audio = false;
    for track in tracks {
        let selected = track.get("selected").and_then(Value::as_bool).unwrap_or(false);
        let lang = track.get("lang").and_then(Value::as_str).map(str::to_string);
        match track.get("type").and_then(Value::as_str) {
            Some("audio") => {
                any_audio = true;
                if selected { choice.audio_enabled = Some(true); choice.audio_language = lang; }
            },
            Some("sub") => {
                any_subtitle = true;
                if selected {
                    choice.subtitles_enabled = Some(true);
                    choice.subtitle_language = lang;
                }
            }
            _ => {}
        }
    }
    if any_audio && choice.audio_enabled.is_none() { choice.audio_enabled = Some(false); }
    if any_subtitle && choice.subtitles_enabled.is_none() {
        choice.subtitles_enabled = Some(false);
    }
    choice
}

impl Player {
    pub fn new(directory: PathBuf) -> Self {
        let (events_tx, _) = broadcast::channel(128);
        Self {
            directory,
            fonts_directory: None,
            session: Arc::new(Mutex::new(None)),
            state: Arc::new(RwLock::new(PlayerState::default())),
            events_tx,
            current_watch: Arc::new(Mutex::new(None)),
            subtitle_selection_epoch: Arc::new(AtomicU64::new(0)),
            req_counter: Arc::new(AtomicU64::new(COMMAND_ID_BASE)),
            queue: Arc::new(RwLock::new(None)),
            policy: Arc::new(RwLock::new(PlaybackPolicy::default())),
            prefs: Arc::new(RwLock::new(PlayerPreferences::default())),
            tracks: Arc::new(RwLock::new(TrackChoice::default())),
            pending_stream: Mutex::new(None),
            speed_hold: Mutex::new(None),
            capabilities: OnceCell::new(),
            launch_args: Vec::new(),
        }
    }

    pub fn with_fonts_directory(mut self, directory: PathBuf) -> Self {
        self.fonts_directory = Some(directory);
        self
    }

    fn custom_font_directory(&self, id: &str) -> Result<PathBuf> {
        let root = self.fonts_directory.as_deref().context("Custom font storage is unavailable")?;
        crate::fonts::font_directory(root, id)
    }

    /// Seed the policy and preferences before the player is shared, so the first playback
    /// already honours the saved settings.
    pub fn with_defaults(mut self, policy: PlaybackPolicy, prefs: PlayerPreferences) -> Self {
        if let Some(slot) = Arc::get_mut(&mut self.policy) {
            *slot.get_mut() = policy;
        }
        if let Some(slot) = Arc::get_mut(&mut self.prefs) {
            *slot.get_mut() = prefs;
        }
        self
    }

    /// Append mpv arguments to every launch, after the defaults so they take precedence. Used
    /// for headless runs (`--vo=null --ao=null --force-window=no`).
    pub fn with_launch_args(mut self, args: Vec<String>) -> Self {
        self.launch_args = args;
        self
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PlayerEvent> {
        self.events_tx.subscribe()
    }

    pub async fn state(&self) -> PlayerState {
        self.state.read().await.clone()
    }

    pub async fn current_watch(&self) -> Option<(WatchEntry, String)> {
        self.current_watch.lock().await.clone()
    }

    pub async fn progress(&self) -> Option<WatchEntry> {
        self.current_watch.lock().await.as_ref().map(|(w, _)| w.clone())
    }

    pub async fn queue(&self) -> Option<PlaybackQueue> {
        self.queue.read().await.clone()
    }

    pub async fn set_queue(&self, queue: Option<PlaybackQueue>) {
        *self.queue.write().await = queue.clone();
        let (has_next, has_prev, q_idx, q_cnt) = if let Some(ref q) = queue {
            (q.has_next(), q.has_prev(), Some(q.current_index), Some(q.episodes.len()))
        } else {
            (false, false, None, None)
        };
        let mut st = self.state.write().await;
        st.has_next = has_next;
        st.has_prev = has_prev;
        st.queue_index = q_idx;
        st.queue_count = q_cnt;
        let cloned_st = st.clone();
        drop(st);
        let _ = self.events_tx.send(PlayerEvent::QueueUpdated { queue });
        let _ = self.events_tx.send(PlayerEvent::StateChanged { state: cloned_st });
    }

    pub async fn policy(&self) -> PlaybackPolicy {
        self.policy.read().await.clone()
    }

    pub async fn set_policy(&self, policy: PlaybackPolicy) {
        *self.policy.write().await = policy;
    }

    pub async fn preferences(&self) -> PlayerPreferences {
        self.prefs.read().await.clone()
    }

    /// The tracks the user currently has selected, as languages that survive a mirror change.
    pub async fn track_choice(&self) -> TrackChoice {
        self.tracks.read().await.clone()
    }

    /// What the installed mpv supports. Probed once per app run.
    pub async fn capabilities(&self) -> PlayerCapabilities {
        self.capabilities.get_or_init(probe_capabilities).await.clone()
    }

    /// Store new preferences. Changes that mpv can apply to a running session (decoder, cache
    /// size, audio compressor) are applied immediately; everything else takes effect on the next
    /// playback. Anything that could not be applied is reported through a `Notice` event.
    pub async fn set_preferences(&self, new: PlayerPreferences) {
        let old = std::mem::replace(&mut *self.prefs.write().await, new.clone());
        let (session_id, active) = {
            let st = self.state.read().await;
            (st.session_id.clone(), st.active)
        };
        let Some(session_id) = session_id.filter(|_| active) else {
            return;
        };
        let caps = self.capabilities().await;
        let mut live: Vec<(&str, &str, Vec<Value>)> = Vec::new();
        if old.subtitle_style.font_id != new.subtitle_style.font_id {
            let selected = new.subtitle_style.font_id.as_deref().map(|id| self.custom_font_directory(id)).transpose();
            match selected {
                Ok(Some(path)) => match path.to_str() {
                    Some(path) => live.push(("subtitle_custom_font", "Custom subtitle font", vec![json!("set_property"), json!("sub-fonts-dir"), json!(path)])),
                    None => self.notice(&session_id, "Custom font path cannot be represented as text".into()),
                },
                Ok(None) => live.push(("subtitle_custom_font", "Custom subtitle font", vec![json!("set_property"), json!("sub-fonts-dir"), json!("")])),
                Err(error) => self.notice(&session_id, format!("Custom subtitle font was not applied: {error:#}")),
            }
        }
        if old.subtitle_style != new.subtitle_style {
            for (name, value) in subtitle_style_properties(&new.subtitle_style) {
                live.push(("subtitle_style", "Subtitle appearance", vec![json!("set_property"), json!(name), value]));
            }
        }
        if old.subtitle_delay != new.subtitle_delay {
            live.push(("subtitle_delay", "Subtitle delay", vec![json!("set_property"), json!("sub-delay"), json!(new.subtitle_delay)]));
        }
        if old.decoding != new.decoding {
            live.push(("hardware_decoding", "Decoding preference", vec![json!("set_property"), json!("hwdec"), json!(new.decoding.mpv_value())]));
        }
        if old.buffer_ram_mb != new.buffer_ram_mb {
            let bytes = if new.buffer_ram_mb == 0 { "150MiB".to_string() } else { format!("{}MiB", new.buffer_ram_mb) };
            live.push(("buffer_size", "Buffer size", vec![json!("set_property"), json!("demuxer-max-bytes"), json!(bytes)]));
        }
        if old.buffer_seconds != new.buffer_seconds {
            let secs = if new.buffer_seconds == 0 { 1.0 } else { f64::from(new.buffer_seconds) };
            live.push(("buffer_time", "Buffer length", vec![json!("set_property"), json!("demuxer-readahead-secs"), json!(secs)]));
        }
        if old.audio_compressor != new.audio_compressor {
            let call = if new.audio_compressor {
                vec![json!("set_property"), json!("af"), json!(COMPRESSOR_FILTER)]
            } else {
                vec![json!("af"), json!("clr"), json!("")]
            };
            live.push(("audio_compressor", "Audio compressor", call));
        }
        let mut reported_unsupported = HashSet::new();
        let mut reported_failures = HashSet::new();
        for (capability, label, call) in live {
            if !caps.supports(capability) {
                if reported_unsupported.insert(capability) {
                    self.notice(&session_id, format!("{label} was not applied: {}", caps.reason(capability).unwrap_or_else(|| "not supported by this mpv".into())));
                }
                continue;
            }
            if let Err(error) = self.command(Value::Array(call)).await {
                if reported_failures.insert(capability) {
                    self.notice(&session_id, format!("{label} could not be fully applied to the running video: {error}"));
                }
            }
        }
    }

    pub fn broadcast_error(&self, session_id: String, message: String) {
        let _ = self.events_tx.send(PlayerEvent::Error { session_id, message });
    }

    pub fn notice(&self, session_id: &str, message: String) {
        let _ = self.events_tx.send(PlayerEvent::Notice {
            session_id: session_id.to_string(),
            message,
        });
    }

    pub fn broadcast(&self, event: PlayerEvent) {
        let _ = self.events_tx.send(event);
    }

    pub async fn play(
        &self,
        stream: &Stream,
        subtitles: &[Subtitle],
        watch: Option<WatchEntry>,
        resume: f64,
        profile_id: Option<&str>,
        queue: Option<PlaybackQueue>,
    ) -> Result<String> {
        self.play_with(stream, subtitles, watch, resume, profile_id, queue, &TrackChoice::default())
            .await
    }

    /// Start a new mpv session. `carry` asks mpv to prefer the same audio/subtitle languages as
    /// the previous stream, which is how a mirror switch keeps the user's track choices.
    #[allow(clippy::too_many_arguments)]
    pub async fn play_with(
        &self,
        stream: &Stream,
        subtitles: &[Subtitle],
        watch: Option<WatchEntry>,
        resume: f64,
        profile_id: Option<&str>,
        queue: Option<PlaybackQueue>,
        carry: &TrackChoice,
    ) -> Result<String> {
        let prepared: Vec<_> = subtitles.iter().cloned().map(|subtitle| PreparedSubtitle { subtitle, cleanup: None }).collect();
        self.play_with_prepared(stream, &prepared, watch, resume, profile_id, queue, carry).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn play_with_prepared(
        &self,
        stream: &Stream,
        subtitles: &[PreparedSubtitle],
        watch: Option<WatchEntry>,
        resume: f64,
        profile_id: Option<&str>,
        queue: Option<PlaybackQueue>,
        carry: &TrackChoice,
    ) -> Result<String> {
        self.launch(stream, subtitles, watch, resume, profile_id, queue, carry, false, false).await
    }

    /// Play a media file from the user's disk. The path is validated here (absolute, resolved,
    /// regular file, media extension), so callers cannot smuggle in anything else.
    pub async fn play_local_file(&self, path: &std::path::Path, resume: f64, profile_id: Option<&str>) -> Result<String> {
        let canonical = crate::entry::checked_local_media(path)?;
        let name = canonical.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Local file".into());
        let stream = Stream { name, url: canonical.display().to_string(), kind: Some(streams::LOCAL_FILE_KIND.into()), stream_type: "VIDEO".into(), ..Default::default() };
        self.launch(&stream, &[], None, resume, profile_id, None, &TrackChoice::default(), true, false).await
    }

    /// Only the app's verified download catalog may call this with an owned local playlist.
    pub(crate) async fn play_offline_hls(&self, path: &std::path::Path, name: &str, subtitles: &[Subtitle], watch: Option<WatchEntry>, resume: f64, profile_id: Option<&str>) -> Result<String> {
        let canonical = std::fs::canonicalize(path).context("Downloaded playlist is unavailable")?;
        if !canonical.is_file() || canonical.file_name().and_then(|name| name.to_str()) != Some("index.m3u8") {
            bail!("Downloaded playlist is unavailable");
        }
        let stream = Stream { name: name.into(), url: canonical.display().to_string(), kind: Some(streams::LOCAL_FILE_KIND.into()), stream_type: "VIDEO".into(), ..Default::default() };
        let prepared: Vec<_> = subtitles.iter().cloned().map(|subtitle| PreparedSubtitle { subtitle, cleanup: None }).collect();
        self.launch(&stream, &prepared, watch, resume, profile_id, None, &TrackChoice::default(), true, true).await
    }

    pub(crate) async fn play_offline_file(&self, path: &std::path::Path, subtitles: &[Subtitle], watch: Option<WatchEntry>, resume: f64, profile_id: Option<&str>) -> Result<String> {
        let canonical = crate::entry::checked_local_media(path)?;
        let name = canonical.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Local file".into());
        let stream = Stream { name, url: canonical.display().to_string(), kind: Some(streams::LOCAL_FILE_KIND.into()), stream_type: "VIDEO".into(), ..Default::default() };
        let prepared: Vec<_> = subtitles.iter().cloned().map(|subtitle| PreparedSubtitle { subtitle, cleanup: None }).collect();
        self.launch(&stream, &prepared, watch, resume, profile_id, None, &TrackChoice::default(), true, false).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn launch(
        &self,
        stream: &Stream,
        subtitles: &[PreparedSubtitle],
        watch: Option<WatchEntry>,
        resume: f64,
        profile_id: Option<&str>,
        queue: Option<PlaybackQueue>,
        carry: &TrackChoice,
        local: bool,
        offline_hls: bool,
    ) -> Result<String> {
        // Defence in depth: the app prepares protected and torrent sources before they get here.
        let mut decryption = Vec::new();
        if !local {
            match crate::drm::requirement(stream) {
                crate::drm::Requirement::UnsupportedDrm(system) => bail!("{}", crate::drm::unsupported_message(&system)),
                crate::drm::Requirement::Torrent => {
                    if !crate::drm::torrent_available() {
                        bail!("{}", crate::drm::TORRENT_UNAVAILABLE);
                    }
                }
                crate::drm::Requirement::ClearKey
                | crate::drm::Requirement::Widevine
                | crate::drm::Requirement::PlayReady
                | crate::drm::Requirement::FairPlay => {
                    decryption = crate::drm::lavf_options(stream)?;
                    if decryption.is_empty() { bail!("The keys for this protected source were not prepared"); }
                }
                crate::drm::Requirement::Ordinary => {}
            }
        }
        let target = if local { stream.url.clone() } else { streams::playback_target(stream)? };
        if !resume.is_finite() || resume < 0.0 {
            bail!("Invalid resume position")
        }
        let mut headers = stream.headers.clone();
        if !stream.referer.is_empty() && !headers.keys().any(|k| k.eq_ignore_ascii_case("referer"))
        {
            headers.insert("Referer".into(), stream.referer.clone());
        }
        for (name, value) in &headers {
            if !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
                || value.contains(['\r', '\n'])
            {
                bail!("Extension returned an invalid HTTP header")
            }
        }
        // mpv's HTTP headers are global. Never silently drop resource headers or apply the
        // video headers to a different subtitle/audio origin; the core must proxy each first.
        if subtitles.iter().any(|s| !s.subtitle.headers.is_empty()) || stream.audio_tracks.iter().any(|a| !a.headers.is_empty()) {
            bail!("Protected subtitle or audio requires a provider media session. Reload the sources and try again");
        }
        if !headers.is_empty() && (!subtitles.is_empty() || !stream.audio_tracks.is_empty()) {
            bail!("Separate audio and subtitles require isolated media sessions when video headers are present");
        }
        for sub in subtitles {
            if local {
                if !std::path::Path::new(&sub.subtitle.url).is_absolute() { bail!("Downloaded subtitle path is invalid"); }
            } else { crate::repository::network_url(&sub.subtitle.url)?; }
        }
        for audio in &stream.audio_tracks { crate::repository::network_url(&audio.url)?; }

        let caps = self.capabilities().await;
        if !caps.available {
            bail!("mpv is unavailable. Install mpv to enable playback");
        }
        if streams::is_playlist(stream) && !caps.supports("playlist_timeline") {
            bail!(
                "{}",
                caps.reason("playlist_timeline")
                    .unwrap_or_else(|| "This mpv cannot play multi-segment playlists".into())
            );
        }

        if let Some(ref q) = queue {
            *self.queue.write().await = Some(q.clone());
            let _ = self.events_tx.send(PlayerEvent::QueueUpdated { queue: Some(q.clone()) });
        }

        self.stop().await;

        let session_id = uuid::Uuid::new_v4().to_string();
        let prof_id = profile_id.unwrap_or("default").to_string();

        let prefs = self.prefs.read().await.clone();
        let policy = self.policy.read().await.clone();
        let (option_args, notes) = launch_options(&prefs, &policy, carry, &caps);

        let private_directory = self
            .directory
            .join(format!("cloudstream-{}", uuid::Uuid::new_v4()));
        let socket = private_directory.join("mpv.sock");
        let mut command = Command::new("mpv");
        command
            .args([
                "--no-config",
                "--idle=yes",
                "--keep-open=yes",
                "--force-window=yes",
                "--terminal=no",
            ])
            .arg(format!("--input-ipc-server={}", socket.display()))
            .arg(format!("--start={resume}"))
            .arg(format!(
                "--title={}",
                watch
                    .as_ref()
                    .map(|w| w.item.name.as_str())
                    .unwrap_or("Cloudstream Desktop")
            ))
            .args(&option_args);
        if offline_hls {
            // The playlist and all its assets were verified in the offline catalog. Even if a
            // file changes after verification, FFmpeg is not allowed to fetch a network URL.
            command.arg("--demuxer-lavf-o=protocol_whitelist=%16%file,crypto,data");
        }
        if let Some(id) = prefs.subtitle_style.font_id.as_deref() {
            if caps.supports("subtitle_custom_font") {
                match self.custom_font_directory(id) {
                    Ok(dir) => match dir.to_str() {
                        Some(path) => { command.arg(format!("--sub-fonts-dir={path}")); },
                        None => self.notice(&session_id, "Custom font path cannot be represented as text".into()),
                    },
                    Err(error) => self.notice(&session_id, format!("Custom subtitle font was not applied: {error:#}")),
                }
            } else {
                self.notice(&session_id, "Custom subtitle fonts are not supported by this mpv installation.".into());
            }
        }
        for (name, value) in headers {
            command.arg(format!("--http-header-fields-append={name}: {value}"));
        }
        for audio in &stream.audio_tracks { command.arg(format!("--audio-file={}", audio.url)); }
        command
            .args(&self.launch_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&private_directory)?;

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                cleanup_socket(&socket);
                *self.state.write().await = PlayerState::default();
                return Err(error).context("mpv is unavailable. Install mpv to enable playback");
            }
        };

        let mut started = false;
        for _ in 0..50 {
            if socket.exists() {
                started = true;
                break;
            }
            if let Ok(Some(_)) = child.try_wait() {
                cleanup_socket(&socket);
                *self.state.write().await = PlayerState::default();
                bail!("mpv exited before playback started");
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        if !started {
            let _ = child.kill().await;
            cleanup_socket(&socket);
            *self.state.write().await = PlayerState::default();
            bail!("mpv did not start its playback connection");
        }

        let stream_conn = match UnixStream::connect(&socket).await {
            Ok(s) => s,
            Err(err) => {
                let _ = child.kill().await;
                cleanup_socket(&socket);
                *self.state.write().await = PlayerState::default();
                return Err(err).context("Could not connect to mpv IPC socket");
            }
        };

        *self.pending_stream.lock().await = Some(streams::active_stream(stream));
        self.start_session_with_subtitles(
            session_id.clone(),
            prof_id,
            Some(child),
            socket,
            stream_conn,
            watch,
            resume,
            subtitles.to_vec(),
            effective_track_choice(&prefs, carry),
        )
        .await;
        // Decryption keys go over the private control socket, never on mpv's command line where
        // any other program of this user could read them. Errors never repeat the option.
        for option in &decryption {
            if self.command(json!(["change-list", "demuxer-lavf-o", "append", option])).await.is_err() {
                self.stop().await;
                bail!("The player could not be given the decryption key");
            }
        }
        // Load only after the IPC reader is connected. A bad URL can fail before mpv's
        // socket is ready, otherwise losing the end-file event needed for failover.
        if let Err(error) = self.command(json!(["loadfile", target, "replace"])).await {
            self.stop().await;
            return Err(error).context("Could not load the selected source in mpv");
        }
        for note in notes {
            self.notice(&session_id, note);
        }

        Ok(session_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start_session(
        &self,
        session_id: String,
        profile_id: String,
        child: Option<Child>,
        socket: PathBuf,
        stream: UnixStream,
        watch: Option<WatchEntry>,
        resume: f64,
    ) {
        self.start_session_with_subtitles(session_id, profile_id, child, socket, stream, watch, resume, Vec::new(), TrackChoice::default()).await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_session_with_subtitles(
        &self,
        session_id: String,
        profile_id: String,
        child: Option<Child>,
        socket: PathBuf,
        stream: UnixStream,
        watch: Option<WatchEntry>,
        resume: f64,
        subtitles: Vec<PreparedSubtitle>,
        carry: TrackChoice,
    ) {
        let (command_tx, command_rx) =
            mpsc::channel::<(Value, oneshot::Sender<Result<Value>>)>(32);
        let external_command_tx = command_tx.clone();
        let pending_commands =
            Arc::new(Mutex::new(HashMap::<u64, oneshot::Sender<Result<Value>>>::new()));
        let (read_half, mut write_half) = stream.into_split();

        let (has_next, has_prev, q_idx, q_cnt) = {
            let q_guard = self.queue.read().await;
            if let Some(ref q) = *q_guard {
                (q.has_next(), q.has_prev(), Some(q.current_index), Some(q.episodes.len()))
            } else {
                (false, false, None, None)
            }
        };

        let prefs = self.prefs.read().await.clone();
        let policy = self.policy.read().await.clone();
        *self.tracks.write().await = TrackChoice::default();
        let active_stream = self.pending_stream.lock().await.take();

        let initial_state = PlayerState {
            session_id: Some(session_id.clone()),
            active: true,
            playing: !policy.start_paused,
            paused: policy.start_paused,
            position: resume,
            duration: 0.0,
            volume: prefs.volume,
            speed: prefs.speed,
            item_name: watch
                .as_ref()
                .map(|w| w.item.name.clone())
                .or_else(|| Some("Direct stream".into())),
            episode_name: watch.as_ref().map(|w| w.episode.name.clone()),
            media_id: watch.as_ref().map(|w| {
                w.item
                    .media_id
                    .clone()
                    .unwrap_or_else(|| compute_media_id(&w.item.provider_id, &w.item.url, w.item.unique_url.as_deref()))
            }),
            episode_id: watch.as_ref().map(|w| w.id.clone()),
            profile_id: Some(profile_id.clone()),
            end_reason: None,
            has_next,
            has_prev,
            queue_index: q_idx,
            queue_count: q_cnt,
            muted: prefs.muted,
            fullscreen: prefs.fullscreen,
            audio_delay: prefs.audio_delay,
            subtitle_delay: prefs.subtitle_delay,
            rotation: 0,
            aspect: prefs.aspect,
            hwdec_current: None,
            stream: active_stream,
            tracks: Vec::new(),
        };
        *self.state.write().await = initial_state.clone();
        if let Some(w) = watch {
            *self.current_watch.lock().await = Some((w, profile_id.clone()));
        } else {
            *self.current_watch.lock().await = None;
        }

        let subtitle_assets = Arc::new(Mutex::new(Vec::new()));
        let assets_for_reader = subtitle_assets.clone();
        let session = ActiveSession {
            subtitle_assets,
            session_id: session_id.clone(),
            profile_id,
            child,
            socket: socket.clone(),
            command_tx,
        };
        *self.session.lock().await = Some(session);
        let session_for_reader = self.session.clone();
        let reader_alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let directory_for_reader = self.directory.clone();
        let selection_epoch = self.subtitle_selection_epoch.clone();
        let initial_selection_epoch = selection_epoch.load(Ordering::SeqCst);

        let _ = self
            .events_tx
            .send(PlayerEvent::StateChanged { state: initial_state });

        let pending_for_writer = pending_commands.clone();
        let counter_for_writer = self.req_counter.clone();
        tokio::spawn(async move {
            for (id, prop) in OBSERVED_PROPERTIES {
                let msg = json!({ "command": ["observe_property", id, prop], "request_id": id })
                    .to_string()
                    + "\n";
                if write_half.write_all(msg.as_bytes()).await.is_err() {
                    return;
                }
            }

            let mut rx = command_rx;
            while let Some((cmd, reply)) = rx.recv().await {
                let req_id = counter_for_writer.fetch_add(1, Ordering::SeqCst);
                pending_for_writer.lock().await.insert(req_id, reply);
                let msg = json!({ "command": cmd, "request_id": req_id }).to_string() + "\n";
                if write_half.write_all(msg.as_bytes()).await.is_err() {
                    if let Some(tx) = pending_for_writer.lock().await.remove(&req_id) {
                        let _ = tx.send(Err(anyhow::anyhow!("Player disconnected")));
                    }
                    break;
                }
            }
        });

        let state_for_reader = self.state.clone();
        let events_tx_for_reader = self.events_tx.clone();
        let watch_for_reader = self.current_watch.clone();
        let pending_for_reader = pending_commands.clone();
        let tracks_for_reader = self.tracks.clone();
        let sid = session_id.clone();
        let sock_path = socket.clone();

        tokio::spawn(async move {
            let mut reader = BufReader::new(read_half).lines();
            let mut pending_subtitles = Some(subtitles);
            // The three properties that together define the aspect mode.
            let (mut keepaspect, mut panscan, mut aspect_override) = (true, 0.0_f64, -1.0_f64);
            while let Ok(Some(line)) = reader.next_line().await {
                let Ok(val) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };

                if let Some(req_id) = val.get("request_id").and_then(Value::as_u64) {
                    if req_id >= COMMAND_ID_BASE {
                        if let Some(tx) = pending_for_reader.lock().await.remove(&req_id) {
                            let err_str =
                                val.get("error").and_then(Value::as_str).unwrap_or("unknown");
                            if err_str == "success" {
                                let data = val.get("data").cloned().unwrap_or(Value::Null);
                                let _ = tx.send(Ok(data));
                            } else {
                                let _ = tx.send(Err(anyhow::anyhow!("Player error: {err_str}")));
                            }
                        }
                    }
                }

                if let Some(event_name) = val.get("event").and_then(Value::as_str) {
                    match event_name {
                        "property-change" => {
                            let prop_name = val.get("name").and_then(Value::as_str).unwrap_or("");
                            let data = val.get("data");
                            let mut state_guard = state_for_reader.write().await;
                            if state_guard.session_id.as_deref() != Some(&sid) {
                                break;
                            }
                            let mut changed = false;
                            match prop_name {
                                "time-pos" => {
                                    if let Some(pos) = data.and_then(Value::as_f64) {
                                        if (state_guard.position - pos).abs() >= 0.1 {
                                            state_guard.position = pos;
                                            state_guard.active = true;
                                            changed = true;
                                            let mut w = watch_for_reader.lock().await;
                                            if let Some((ref mut entry, _)) = *w {
                                                entry.position = pos;
                                                entry.watched_at = timestamp_ms();
                                            }
                                        }
                                    }
                                }
                                "duration" => {
                                    if let Some(dur) = data.and_then(Value::as_f64) {
                                        if dur > 0.0 && (state_guard.duration - dur).abs() >= 0.1 {
                                            state_guard.duration = dur;
                                            changed = true;
                                            let mut w = watch_for_reader.lock().await;
                                            if let Some((ref mut entry, _)) = *w {
                                                entry.duration = dur;
                                            }
                                        }
                                    }
                                }
                                "pause" => {
                                    if let Some(p) = data.and_then(Value::as_bool) {
                                        if state_guard.paused != p {
                                            state_guard.paused = p;
                                            if state_guard.active {
                                                state_guard.playing = !p;
                                            }
                                            changed = true;
                                        }
                                    }
                                }
                                "volume" => {
                                    if let Some(vol) = data.and_then(Value::as_f64) {
                                        if (state_guard.volume - vol).abs() >= 0.1 {
                                            state_guard.volume = vol;
                                            changed = true;
                                        }
                                    }
                                }
                                "speed" => {
                                    if let Some(spd) = data.and_then(Value::as_f64) {
                                        if (state_guard.speed - spd).abs() >= 0.01 {
                                            state_guard.speed = spd;
                                            changed = true;
                                        }
                                    }
                                }
                                "mute" => {
                                    if let Some(m) = data.and_then(Value::as_bool) {
                                        if state_guard.muted != m {
                                            state_guard.muted = m;
                                            changed = true;
                                        }
                                    }
                                }
                                "fullscreen" => {
                                    if let Some(f) = data.and_then(Value::as_bool) {
                                        if state_guard.fullscreen != f {
                                            state_guard.fullscreen = f;
                                            changed = true;
                                        }
                                    }
                                }
                                "audio-delay" => {
                                    if let Some(d) = data.and_then(Value::as_f64) {
                                        if (state_guard.audio_delay - d).abs() >= 0.0005 {
                                            state_guard.audio_delay = d;
                                            changed = true;
                                        }
                                    }
                                }
                                "sub-delay" => {
                                    if let Some(d) = data.and_then(Value::as_f64) {
                                        if (state_guard.subtitle_delay - d).abs() >= 0.0005 {
                                            state_guard.subtitle_delay = d;
                                            changed = true;
                                        }
                                    }
                                }
                                "video-rotate" => {
                                    if let Some(r) = data.and_then(Value::as_i64) {
                                        let r = r.rem_euclid(360) as i32;
                                        if state_guard.rotation != r {
                                            state_guard.rotation = r;
                                            changed = true;
                                        }
                                    }
                                }
                                "panscan" | "keepaspect" | "video-aspect-override" => {
                                    match prop_name {
                                        "panscan" => panscan = data.and_then(Value::as_f64).unwrap_or(panscan),
                                        "keepaspect" => keepaspect = data.and_then(Value::as_bool).unwrap_or(keepaspect),
                                        _ => aspect_override = data.and_then(Value::as_f64).unwrap_or(aspect_override),
                                    }
                                    let mode = AspectMode::from_mpv(keepaspect, panscan, aspect_override);
                                    if state_guard.aspect != mode {
                                        state_guard.aspect = mode;
                                        changed = true;
                                    }
                                }
                                "hwdec-current" => {
                                    let current = data
                                        .and_then(Value::as_str)
                                        .filter(|d| !d.is_empty() && *d != "no")
                                        .map(str::to_string);
                                    if state_guard.hwdec_current != current {
                                        state_guard.hwdec_current = current;
                                        changed = true;
                                    }
                                }
                                "track-list" => {
                                    if let Some(list) = data {
                                        *tracks_for_reader.write().await = track_choice_from(list);
                                        let tracks = player_tracks_from(list);
                                        if state_guard.tracks != tracks {
                                            state_guard.tracks = tracks;
                                            changed = true;
                                        }
                                    }
                                }
                                _ => {}
                            }
                            if changed {
                                let st = state_guard.clone();
                                drop(state_guard);
                                let _ = events_tx_for_reader.send(PlayerEvent::StateChanged {
                                    state: st,
                                });
                            }
                        }
                        "file-loaded" => {
                            let mut state_guard = state_for_reader.write().await;
                            if state_guard.session_id.as_deref() == Some(&sid) {
                                state_guard.active = true;
                                state_guard.playing = !state_guard.paused;
                                let st = state_guard.clone();
                                drop(state_guard);
                                if let Some(subtitles) = pending_subtitles.take() {
                                    let command_tx = external_command_tx.clone();
                                    let events = events_tx_for_reader.clone();
                                    let subtitle_session = sid.clone();
                                    let carry = carry.clone();
                                    let session = session_for_reader.clone();
                                    let assets = assets_for_reader.clone();
                                    let directory = directory_for_reader.clone();
                                    let selection_epoch = selection_epoch.clone();
                                    let alive = reader_alive.clone();
                                    tokio::spawn(async move {
                                        load_provider_subtitles(subtitles, carry, command_tx, events, subtitle_session, session, assets, directory, selection_epoch, initial_selection_epoch, alive).await;
                                    });
                                }
                                let _ = events_tx_for_reader.send(PlayerEvent::FileLoaded {
                                    session_id: sid.clone(),
                                });
                                let _ = events_tx_for_reader.send(PlayerEvent::StateChanged {
                                    state: st,
                                });
                            }
                        }
                        "end-file" => {
                            let reason = val
                                .get("reason")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown")
                                .to_string();
                            let detail = val
                                .get("file_error")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                            let mut state_guard = state_for_reader.write().await;
                            if state_guard.session_id.as_deref() == Some(&sid) {
                                state_guard.playing = false;
                                state_guard.end_reason = Some(reason.clone());
                                let st = state_guard.clone();
                                drop(state_guard);
                                let _ = events_tx_for_reader.send(PlayerEvent::EndFile {
                                    session_id: sid.clone(),
                                    reason,
                                    detail,
                                });
                                let _ = events_tx_for_reader.send(PlayerEvent::StateChanged {
                                    state: st,
                                });
                            }
                        }
                        "shutdown" => {
                            let mut state_guard = state_for_reader.write().await;
                            if state_guard.session_id.as_deref() == Some(&sid) {
                                state_guard.active = false;
                                state_guard.playing = false;
                                state_guard.paused = false;
                                let st = state_guard.clone();
                                drop(state_guard);
                                let _ = events_tx_for_reader.send(PlayerEvent::Shutdown {
                                    session_id: sid.clone(),
                                });
                                let _ = events_tx_for_reader.send(PlayerEvent::StateChanged {
                                    state: st,
                                });
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }

            {
                let mut state_guard = state_for_reader.write().await;
                if state_guard.session_id.as_deref() == Some(&sid) {
                    state_guard.active = false;
                    state_guard.playing = false;
                    state_guard.paused = false;
                    let st = state_guard.clone();
                    drop(state_guard);
                    let _ = events_tx_for_reader.send(PlayerEvent::Shutdown {
                        session_id: sid.clone(),
                    });
                    let _ = events_tx_for_reader.send(PlayerEvent::StateChanged {
                        state: st,
                    });
                }
            }

            reader_alive.store(false, Ordering::SeqCst);
            cleanup_socket(&sock_path);
            cleanup_subtitle_assets(&assets_for_reader).await;

            for (_, tx) in pending_for_reader.lock().await.drain() {
                let _ = tx.send(Err(anyhow::anyhow!("Player disconnected")));
            }
        });
    }

    pub async fn command(&self, command: Value) -> Result<Value> {
        self.command_for_session(command, None).await
    }

    async fn command_for_session(&self, command: Value, expected_session: Option<&str>) -> Result<Value> {
        let (tx, rx) = oneshot::channel();
        let command_tx = {
            let guard = self.session.lock().await;
            let Some(ref playback) = *guard else {
                bail!("No active playback");
            };
            if expected_session.is_some_and(|id| id != playback.session_id) {
                bail!("Playback changed before the action could be applied.");
            }
            playback.command_tx.clone()
        };
        command_tx
            .send((command, tx))
            .await
            .map_err(|_| anyhow::anyhow!("Failed to send player command: player is shutting down"))?;
        tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .context("Player response timed out")?
            .context("Player dropped response channel")?
    }

    /// Apply a manual skip only to the session and interval the person actually saw.
    pub async fn skip_interval(&self, expected_session: &str, start: f64, end: f64) -> Result<()> {
        let state = self.state().await;
        if !state.active || state.session_id.as_deref() != Some(expected_session) { bail!("Playback changed; the skip was not applied"); }
        if !start.is_finite() || !end.is_finite() || !state.duration.is_finite() || start < 0.0 || end <= start || end > state.duration
            || state.position < start || state.position >= end { bail!("This skip is no longer available"); }
        self.command_for_session(json!(["seek", end, "absolute+exact"]), Some(expected_session)).await?;
        Ok(())
    }

    /// Temporarily change speed for a held key without modifying saved playback preferences.
    pub async fn begin_speed_hold(&self, expected_session: &str, speed: f64) -> Result<()> {
        if !speed.is_finite() || !(1.0..=SPEED_MAX).contains(&speed) { bail!("Hold speed is out of range"); }
        let mut hold = self.speed_hold.lock().await;
        if hold.as_ref().is_some_and(|(id, _)| id == expected_session) { return Ok(()); }
        let state = self.state().await;
        if !state.active || state.session_id.as_deref() != Some(expected_session) { bail!("Playback changed; speed was not changed"); }
        self.command_for_session(json!(["set_property", "speed", speed]), Some(expected_session)).await?;
        *hold = Some((expected_session.to_owned(), state.speed));
        Ok(())
    }

    /// Restore only the session which started the hold, never a replacement player session.
    pub async fn end_speed_hold(&self, expected_session: &str) -> Result<()> {
        let mut hold = self.speed_hold.lock().await;
        if hold.as_ref().is_none_or(|(id, _)| id != expected_session) { return Ok(()); }
        let (_, speed) = hold.take().unwrap();
        self.command_for_session(json!(["set_property", "speed", speed]), Some(expected_session)).await?;
        Ok(())
    }

    /// Load a user-selected subtitle without modifying the source file or restarting playback.
    pub async fn add_local_subtitle(&self, path: &std::path::Path, expected_session: &str) -> Result<()> {
        let canonical = std::fs::canonicalize(path).context("Could not open the subtitle file")?;
        let metadata = std::fs::metadata(&canonical).context("Could not inspect the subtitle file")?;
        if !metadata.is_file() { bail!("Choose a subtitle file, not a folder"); }
        if metadata.len() > crate::subtitles::MAX_SUBTITLE_BYTES { bail!("Subtitle files must be 20 MiB or smaller"); }
        let supported = crate::subtitles::supported_subtitle(&canonical);
        if !supported { bail!("Choose an SRT, VTT, ASS, SSA, TTML, DFXP or SAMI subtitle file"); }
        let title = canonical.file_name().and_then(|name| name.to_str()).unwrap_or("Local subtitle");
        self.subtitle_selection_epoch.fetch_add(1, Ordering::SeqCst);
        let transforms = self.preferences().await.subtitle_transforms;
        if !transforms.is_empty() {
            use std::io::Read;
            let source = std::fs::File::open(&canonical).context("Could not open the subtitle file")?;
            let mut bytes = Vec::new();
            source.take(crate::subtitles::MAX_SUBTITLE_BYTES + 1).read_to_end(&mut bytes).context("Could not read the subtitle file")?;
            let extension = canonical.extension().and_then(|x| x.to_str()).unwrap_or("");
            let copy = crate::subtitles::transform_text(&bytes, extension, &transforms)?;
            return self.add_subtitle_copy(&copy, extension, title, expected_session).await;
        }
        let path = canonical.to_str().context("This subtitle filename cannot be represented as text")?;
        self.command_for_session(json!(["sub-add", path, "select", title]), Some(expected_session)).await
            .map_err(|error| anyhow::anyhow!("Could not load the selected subtitle: {error:#}"))?;
        Ok(())
    }

    /// Extract the selected archive member to a private session-owned directory and load it.
    pub async fn add_archived_subtitle(&self, bytes: &[u8], index: usize, expected_session: &str) -> Result<()> {
        self.subtitle_selection_epoch.fetch_add(1, Ordering::SeqCst);
        let (entry, contents) = crate::subtitles::extract_archive_entry(bytes, index)?;
        let extension = std::path::Path::new(&entry.name).extension().and_then(|x| x.to_str())
            .context("The subtitle has no supported extension")?;
        let transforms = self.preferences().await.subtitle_transforms;
        let contents = crate::subtitles::transform_text(&contents, extension, &transforms)?;
        let title = std::path::Path::new(&entry.name).file_name().and_then(|x| x.to_str()).unwrap_or("Archived subtitle");
        self.add_subtitle_copy(&contents, extension, title, expected_session).await
    }

    /// Load subtitle text that was downloaded by the app (an online service), through the same
    /// private, session-owned copy as an archive member, with the person's text transforms applied.
    pub async fn add_subtitle_bytes(&self, bytes: &[u8], extension: &str, title: &str, expected_session: &str) -> Result<()> {
        if bytes.len() as u64 > crate::subtitles::MAX_SUBTITLE_BYTES { bail!("Subtitle files must be 20 MiB or smaller"); }
        if !matches!(extension, "srt" | "vtt" | "ass" | "ssa" | "ttml" | "dfxp" | "smi" | "sami") { bail!("That subtitle format is not supported"); }
        self.subtitle_selection_epoch.fetch_add(1, Ordering::SeqCst);
        let transforms = self.preferences().await.subtitle_transforms;
        let contents = crate::subtitles::transform_text(bytes, extension, &transforms)?;
        let title: String = title.chars().filter(|c| !c.is_control()).take(80).collect();
        self.add_subtitle_copy(&contents, extension, &title, expected_session).await
    }

    /// Keep generated subtitle copies private and alive for the owning playback session.
    async fn add_subtitle_copy(&self, contents: &[u8], extension: &str, title: &str, expected_session: &str) -> Result<()> {
        let dir = self.directory.join(format!("cloudstream-subtitle-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).context("Could not create a temporary subtitle folder")?;
        let file = dir.join(format!("subtitle.{extension}"));
        if let Err(error) = std::fs::write(&file, contents) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(error).context("Could not extract the subtitle");
        }
        let assets = {
            let guard = self.session.lock().await;
            match guard.as_ref() {
                Some(session) if session.session_id == expected_session => session.subtitle_assets.clone(),
                _ => {
                    let _ = std::fs::remove_dir_all(&dir);
                    bail!("Playback changed while choosing the subtitle. Choose the file again.");
                }
            }
        };
        // Recheck ownership while holding the session lock so stop cannot race registration.
        {
            let guard = self.session.lock().await;
            if guard.as_ref().is_none_or(|session| session.session_id != expected_session) {
                let _ = std::fs::remove_dir_all(&dir);
                bail!("Playback changed while choosing the subtitle. Choose the file again.");
            }
            assets.lock().await.push(dir.clone());
        }
        let path = file.to_str().context("Temporary subtitle path cannot be represented as text")?;
        if let Err(error) = self.command_for_session(json!(["sub-add", path, "select", title]), Some(expected_session)).await {
            assets.lock().await.retain(|item| item != &dir);
            let _ = std::fs::remove_dir_all(&dir);
            return Err(anyhow::anyhow!("Could not load the subtitle copy: {error:#}"));
        }
        Ok(())
    }

    /// Validate and run a typed command. Returns the updated preferences when the command
    /// changed one that should persist across videos (volume, mute, speed, fullscreen, aspect,
    /// audio delay); rotation and pause/seek are per video and never persisted.
    pub async fn execute(&self, command: PlayerCommand) -> Result<Option<PlayerPreferences>> {
        let state = self.state().await;
        if !state.active {
            bail!("Nothing is playing");
        }
        command.validate(state.duration)?;
        let selection = match &command {
            PlayerCommand::SetAudioTrack { id: Some(id) } => Some(("audio", *id)),
            PlayerCommand::SetSubtitleTrack { id: Some(id) } => Some(("sub", *id)),
            _ => None,
        };
        if let Some((kind, id)) = selection {
            if !state.tracks.iter().any(|track| track.kind == kind && track.id == id) {
                bail!("That {kind} track is no longer available");
            }
        }
        if matches!(&command, PlayerCommand::SetSubtitleTrack { .. }) { self.subtitle_selection_epoch.fetch_add(1, Ordering::SeqCst); }
        for call in command.to_mpv() {
            self.command(Value::Array(call))
                .await
                .map_err(|e| friendly_error(&command, &e.to_string()))?;
        }
        let mut prefs = self.prefs.write().await;
        let persisted = match &command {
            PlayerCommand::SetVolume { volume } => {
                prefs.volume = *volume;
                true
            }
            PlayerCommand::SetMute { muted } => {
                prefs.muted = *muted;
                true
            }
            PlayerCommand::SetSpeed { speed } => {
                prefs.speed = *speed;
                true
            }
            PlayerCommand::SetFullscreen { fullscreen } => {
                prefs.fullscreen = *fullscreen;
                true
            }
            PlayerCommand::SetAspect { mode } => {
                prefs.aspect = *mode;
                true
            }
            PlayerCommand::SetAudioDelay { seconds } => {
                prefs.audio_delay = *seconds;
                true
            }
            PlayerCommand::SetSubtitleDelay { seconds } => {
                prefs.subtitle_delay = *seconds;
                true
            }
            PlayerCommand::SetSubtitleTrack { id } => {
                prefs.subtitles_enabled = Some(id.is_some());
                if let Some(id) = id { prefs.subtitle_language = state.tracks.iter().find(|t| t.kind == "sub" && t.id == *id).and_then(|t| t.language.clone()).filter(|l| safe_language(l)); }
                true
            }
            PlayerCommand::SetAudioTrack { id } => {
                prefs.audio_enabled = Some(id.is_some());
                if let Some(id) = id { prefs.audio_language = state.tracks.iter().find(|t| t.kind == "audio" && t.id == *id).and_then(|t| t.language.clone()).filter(|l| safe_language(l)); }
                true
            }
            _ => false,
        };
        Ok(persisted.then(|| prefs.clone()))
    }

    pub async fn stop(&self) {
        *self.speed_hold.lock().await = None;
        let session = self.session.lock().await.take();
        if let Some(mut active) = session {
            if let Some(mut child) = active.child.take() {
                let _ = child.kill().await;
            }
            cleanup_socket(&active.socket);
            cleanup_subtitle_assets(&active.subtitle_assets).await;
            {
                let mut state_guard = self.state.write().await;
                if state_guard.session_id.as_deref() == Some(&active.session_id) {
                    state_guard.active = false;
                    state_guard.playing = false;
                    state_guard.paused = false;
                    let st = state_guard.clone();
                    drop(state_guard);
                    let _ = self.events_tx.send(PlayerEvent::Shutdown {
                        session_id: active.session_id,
                    });
                    let _ = self.events_tx.send(PlayerEvent::StateChanged {
                        state: st,
                    });
                }
            }
        }
    }
}

pub fn cleanup_socket(socket: &std::path::Path) {
    let _ = std::fs::remove_file(socket);
    if let Some(parent) = socket.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

async fn cleanup_subtitle_assets(assets: &Mutex<Vec<PathBuf>>) {
    for dir in assets.lock().await.drain(..) { let _ = std::fs::remove_dir_all(dir); }
}

/// Prepare provider subtitle copies after file-loaded so network delays never block the video.
/// All writes and commands are tied to the owning session; dropping a session cancels requests.
async fn load_provider_subtitles(
    mut subtitles: Vec<PreparedSubtitle>,
    carry: TrackChoice,
    command_tx: mpsc::Sender<(Value, oneshot::Sender<Result<Value>>)>,
    events: broadcast::Sender<PlayerEvent>,
    session_id: String,
    session: Arc<Mutex<Option<ActiveSession>>>,
    assets: Arc<Mutex<Vec<PathBuf>>>,
    directory: PathBuf,
    selection_epoch: Arc<AtomicU64>,
    initial_selection_epoch: u64,
    alive: Arc<std::sync::atomic::AtomicBool>,
) {
    let mut pending = subtitles.iter_mut().enumerate().filter_map(|(i, subtitle)| {
        subtitle.cleanup.take().map(|(format, no_proxy, options)| (i, subtitle.subtitle.url.clone(), format, no_proxy, options))
    }).collect::<Vec<_>>().into_iter();
    let mut running = tokio::task::JoinSet::new();
    let mut copies = HashMap::new();
    let mut total = 0usize;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    let notice = |message: String| { let _ = events.send(PlayerEvent::Notice { session_id: session_id.clone(), message }); };
    loop {
        while running.len() < 4 {
            let Some((index, url, format, no_proxy, options)) = pending.next() else { break };
            running.spawn(async move {
                let result = async {
                    let bytes = crate::subtitles::fetch_for_cleanup(&url, no_proxy).await?;
                    crate::subtitles::transform_text(&bytes, &format, &options)
                }.await;
                (index, format, result)
            });
        }
        tokio::select! {
            result = running.join_next() => match result {
                None => break,
                Some(Ok((index, format, Ok(bytes)))) if total.saturating_add(bytes.len()) <= 40 * 1024 * 1024 => {
                    total += bytes.len();
                    copies.insert(index, (format, bytes));
                },
                Some(Ok((index, _, _))) => notice(format!("Could not clean subtitle {}. Using the original.", subtitles[index].subtitle.lang)),
                Some(Err(_)) => notice("Could not clean a provider subtitle. Using the original.".into()),
            },
            _ = tokio::time::sleep_until(deadline) => {
                notice("Subtitle cleanup timed out for some tracks. Using the originals.".into());
                break;
            },
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                if !alive.load(Ordering::SeqCst) || session.lock().await.as_ref().is_none_or(|s| s.session_id != session_id) { return; }
            },
        }
    }
    running.abort_all();
    let mut selected_preferred = false;
    for (index, prepared) in subtitles.into_iter().enumerate() {
        let mut subtitle = prepared.subtitle;
        let guard = session.lock().await;
        if !alive.load(Ordering::SeqCst) || guard.as_ref().is_none_or(|s| s.session_id != session_id) { return; }
        if let Some((format, bytes)) = copies.remove(&index) {
            // Format is derived from a fixed allowlist, never directly from a provider path.
            if matches!(format.as_str(), "srt" | "vtt" | "ass" | "ssa") {
                let mut owned = assets.lock().await;
                if !alive.load(Ordering::SeqCst) { return; }
                let dir = directory.join(format!("cloudstream-subtitle-{}", uuid::Uuid::new_v4()));
                let path = dir.join(format!("subtitle.{format}"));
                let written = std::fs::DirBuilder::new().mode(0o700).create(&dir).and_then(|_| std::fs::write(&path, bytes));
                match (written, path.to_str()) {
                    (Ok(()), Some(path)) => { owned.push(dir); subtitle.url = path.into(); }
                    _ => { let _ = std::fs::remove_dir_all(&dir); notice("Could not prepare a cleaned provider subtitle; using the original.".into()); }
                }
            }
        }
        let language = subtitle.lang_tag.as_deref().filter(|tag| safe_language(tag)).unwrap_or("");
        let selected = !selected_preferred && selection_epoch.load(Ordering::SeqCst) == initial_selection_epoch && carry.subtitles_enabled != Some(false)
            && carry.subtitle_language.as_deref().is_some_and(|wanted| !language.is_empty() && wanted.eq_ignore_ascii_case(language));
        let flags = if selected { "select" } else { "auto" };
        let command = json!(["sub-add", subtitle.url, flags, subtitle.lang, language]);
        let (reply, receive) = oneshot::channel();
        let sent = command_tx.send((command, reply)).await.is_ok();
        drop(guard);
        let loaded = sent && matches!(tokio::time::timeout(std::time::Duration::from_secs(15), receive).await, Ok(Ok(Ok(_))));
        if loaded && selected { selected_preferred = true; }
        if !loaded { notice("An external subtitle could not be loaded.".into()); }
    }
}
