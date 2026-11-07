//! MPRIS media-player integration (T27): lets desktop media keys, panel widgets and tools such as
//! `playerctl` see and control playback.
//!
//! The service is a thin bridge. It owns no playback state; the app pushes the player's
//! authoritative state in (`update`), and controls arriving over D-Bus become `MprisRequest`s on a
//! channel that the app maps onto the same validated player commands the UI uses.
//!
//! Safety: `OpenUri` from another process is handed to the normal entry parser and still needs the
//! user's confirmation; `Quit` is not offered; seeking is bounded by the known duration.
//! mpv's own MPRIS script is not loaded (`--no-config`), so only this service owns the bus name.
use anyhow::{Context, Result};
use std::collections::HashMap;
use tokio::sync::mpsc;
use zbus::{
    connection::Builder,
    interface,
    object_server::SignalEmitter,
    zvariant::{ObjectPath, OwnedValue, Value},
};

pub const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
pub const BUS_NAME: &str = "org.mpris.MediaPlayer2.cloudstream_desktop";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    #[default]
    Stopped,
}

impl PlaybackStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Playing => "Playing",
            Self::Paused => "Paused",
            Self::Stopped => "Stopped",
        }
    }
}

/// What the desktop should see. Built from the player's state by `MprisState::from_player`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MprisState {
    pub status: PlaybackStatus,
    pub title: Option<String>,
    pub album: Option<String>,
    /// Microseconds, if the duration is known.
    pub length_us: Option<i64>,
    pub position_us: i64,
    /// 0.0 to 1.0 (mpv's 0 to 100).
    pub volume: f64,
    pub rate: f64,
    pub can_go_next: bool,
    pub can_go_previous: bool,
    pub track_id: Option<String>,
}

/// The MPRIS track id for a player session (letters and digits only, as D-Bus requires).
pub fn track_id_for(session_id: &str) -> String {
    format!("/dev/cloudstream/desktop/track/{}", session_id.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>())
}

impl MprisState {
    pub fn from_player(state: &crate::models::PlayerState) -> Self {
        if !state.active {
            return Self { volume: (state.volume / 100.0).clamp(0.0, 1.0), rate: 1.0, ..Default::default() };
        }
        let (title, album) = match (&state.episode_name, &state.item_name) {
            (Some(episode), Some(item)) if !episode.is_empty() => (Some(episode.clone()), Some(item.clone())),
            (_, item) => (item.clone(), None),
        };
        // Track ids must be valid D-Bus object path elements: letters, digits and underscores.
        let track_id = state.session_id.as_deref().map(track_id_for);
        Self {
            status: if state.paused { PlaybackStatus::Paused } else { PlaybackStatus::Playing },
            title,
            album,
            length_us: (state.duration > 0.0).then(|| (state.duration * 1_000_000.0) as i64),
            position_us: (state.position.max(0.0) * 1_000_000.0) as i64,
            volume: (state.volume / 100.0).clamp(0.0, 1.0),
            rate: if state.speed > 0.0 { state.speed } else { 1.0 },
            can_go_next: state.has_next,
            can_go_previous: state.has_prev,
            track_id,
        }
    }

    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let mut map: HashMap<String, OwnedValue> = HashMap::new();
        let mut put = |key: &str, value: Value<'_>| {
            if let Ok(owned) = OwnedValue::try_from(value) {
                map.insert(key.to_string(), owned);
            }
        };
        let id = self.track_id.as_deref().unwrap_or(NO_TRACK);
        if let Ok(path) = ObjectPath::try_from(id) {
            put("mpris:trackid", Value::from(path));
        }
        if self.status != PlaybackStatus::Stopped {
            if let Some(length) = self.length_us {
                put("mpris:length", Value::from(length));
            }
            if let Some(title) = &self.title {
                put("xesam:title", Value::from(title.as_str()));
            }
            if let Some(album) = &self.album {
                put("xesam:album", Value::from(album.as_str()));
            }
        }
        map
    }
}

/// A control arriving from the desktop.
#[derive(Debug, Clone, PartialEq)]
pub enum MprisRequest {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    /// Relative seek in microseconds.
    Seek(i64),
    /// Absolute position for the named track, in microseconds.
    SetPosition { track_id: String, position_us: i64 },
    SetVolume(f64),
    SetRate(f64),
    Raise,
    /// Another process asked the app to open an address or file. Never acted on without asking.
    OpenUri(String),
}

struct Root {
    tx: mpsc::UnboundedSender<MprisRequest>,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {
        let _ = self.tx.send(MprisRequest::Raise);
    }
    /// Quitting the app from another process is deliberately not offered.
    fn quit(&self) {}

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> String {
        "Cloudstream Desktop".into()
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "cloudstream-desktop".into()
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["http".into(), "https".into(), "file".into()]
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct PlayerInterface {
    tx: mpsc::UnboundedSender<MprisRequest>,
    state: MprisState,
}

impl PlayerInterface {
    fn send(&self, request: MprisRequest) {
        let _ = self.tx.send(request);
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerInterface {
    fn next(&self) {
        if self.state.can_go_next {
            self.send(MprisRequest::Next);
        }
    }
    fn previous(&self) {
        if self.state.can_go_previous {
            self.send(MprisRequest::Previous);
        }
    }
    fn pause(&self) {
        self.send(MprisRequest::Pause);
    }
    fn play_pause(&self) {
        self.send(MprisRequest::PlayPause);
    }
    fn stop(&self) {
        self.send(MprisRequest::Stop);
    }
    fn play(&self) {
        self.send(MprisRequest::Play);
    }
    fn seek(&self, offset: i64) {
        if self.state.status != PlaybackStatus::Stopped {
            self.send(MprisRequest::Seek(offset));
        }
    }
    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        if self.state.status != PlaybackStatus::Stopped {
            self.send(MprisRequest::SetPosition { track_id: track_id.to_string(), position_us: position });
        }
    }
    fn open_uri(&self, uri: String) {
        if uri.len() <= crate::entry::MAX_ARGUMENT_BYTES {
            self.send(MprisRequest::OpenUri(uri));
        }
    }

    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.state.status.as_str().into()
    }
    #[zbus(property)]
    fn loop_status(&self) -> String {
        "None".into()
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        self.state.rate
    }
    #[zbus(property)]
    fn set_rate(&mut self, rate: f64) {
        if rate.is_finite() && rate > 0.0 {
            self.send(MprisRequest::SetRate(rate));
        }
    }
    #[zbus(property)]
    fn shuffle(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        self.state.metadata()
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.state.volume
    }
    #[zbus(property)]
    fn set_volume(&mut self, volume: f64) {
        if volume.is_finite() {
            self.send(MprisRequest::SetVolume(volume.clamp(0.0, 1.0)));
        }
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        self.state.position_us
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        crate::models::SPEED_MIN
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        crate::models::SPEED_MAX
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.state.can_go_next
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.state.can_go_previous
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.state.status != PlaybackStatus::Stopped
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.state.status != PlaybackStatus::Stopped
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.state.status != PlaybackStatus::Stopped
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

pub struct MprisService {
    connection: zbus::Connection,
    last: tokio::sync::Mutex<MprisState>,
}

impl MprisService {
    /// Register on the user's session bus.
    pub async fn start(tx: mpsc::UnboundedSender<MprisRequest>) -> Result<Self> {
        Self::start_with(Builder::session().context("No D-Bus session bus is available")?, tx).await
    }

    /// Register on a specific bus address (used by tests with a private bus).
    pub async fn start_on(address: &str, tx: mpsc::UnboundedSender<MprisRequest>) -> Result<Self> {
        Self::start_with(Builder::address(address).context("Invalid D-Bus address")?, tx).await
    }

    async fn start_with(builder: Builder<'_>, tx: mpsc::UnboundedSender<MprisRequest>) -> Result<Self> {
        let initial = MprisState { rate: 1.0, ..Default::default() };
        let connection = builder
            .serve_at(OBJECT_PATH, Root { tx: tx.clone() })?
            .serve_at(OBJECT_PATH, PlayerInterface { tx, state: initial.clone() })?
            .build()
            .await
            .context("Could not connect to D-Bus to register the media player")?;
        // Do not queue behind another owner: two players must never share one MPRIS identity.
        connection
            .request_name_with_flags(BUS_NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
            .await
            .context("Could not register the media player on D-Bus. Another instance may already own the name")?;
        Ok(Self { connection, last: tokio::sync::Mutex::new(initial) })
    }

    /// Push the player's state. Only properties that changed are announced.
    pub async fn update(&self, new: MprisState) -> Result<()> {
        let mut last = self.last.lock().await;
        if *last == new {
            return Ok(());
        }
        let iface_ref = self.connection.object_server().interface::<_, PlayerInterface>(OBJECT_PATH).await?;
        let before = last.clone();
        {
            let mut iface = iface_ref.get_mut().await;
            iface.state = new.clone();
        }
        let iface = iface_ref.get().await;
        let emitter = iface_ref.signal_emitter();
        if before.status != new.status {
            iface.playback_status_changed(emitter).await?;
            iface.can_play_changed(emitter).await?;
            iface.can_pause_changed(emitter).await?;
            iface.can_seek_changed(emitter).await?;
        }
        if before.title != new.title || before.album != new.album || before.length_us != new.length_us || before.track_id != new.track_id || before.status != new.status {
            iface.metadata_changed(emitter).await?;
        }
        if (before.volume - new.volume).abs() > f64::EPSILON {
            iface.volume_changed(emitter).await?;
        }
        if (before.rate - new.rate).abs() > f64::EPSILON {
            iface.rate_changed(emitter).await?;
        }
        if before.can_go_next != new.can_go_next {
            iface.can_go_next_changed(emitter).await?;
        }
        if before.can_go_previous != new.can_go_previous {
            iface.can_go_previous_changed(emitter).await?;
        }
        // A jump (not ordinary playback) is announced so widgets resync their position.
        if (new.position_us - before.position_us).abs() > 2_000_000 && before.track_id == new.track_id {
            PlayerInterface::seeked(emitter, new.position_us).await?;
        }
        *last = new;
        Ok(())
    }
}
