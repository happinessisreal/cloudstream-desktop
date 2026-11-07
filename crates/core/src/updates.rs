//! App update availability (T32).
//!
//! This only *finds out* whether a newer, properly signed release exists and describes it. It
//! never downloads or installs anything and never touches Android releases: installing is left to
//! the user's package manager or to a download they choose to make, checked against the signed
//! SHA-256 shown here.
//!
//! The feed is an envelope `{ "payload": <base64 JSON>, "signature": <base64 ed25519> }`. The
//! signature covers the exact payload bytes, so there is no re-encoding step to disagree about.
//! A build without a configured public key and feed address reports "not configured" instead of
//! pretending to check.
use anyhow::{bail, Context, Result};
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_FEED_BYTES: usize = 256 * 1024;
const MAX_PACKAGE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const PACKAGE_KINDS: [&str; 3] = ["deb", "appimage", "tarball"];

/// The release-signing public key, supplied when building a release (`CLOUDSTREAM_UPDATE_PUBLIC_KEY`,
/// 32 bytes as base64). Absent in development builds.
pub fn configured_public_key() -> Option<[u8; 32]> {
    let text = option_env!("CLOUDSTREAM_UPDATE_PUBLIC_KEY")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(text.trim()).ok()?;
    bytes.try_into().ok()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    #[serde(default)]
    pub channel: UpdateChannel,
    /// The release feed address (HTTPS). Empty when this build has no update channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed_url: Option<String>,
    /// A version the user chose not to be told about again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_version: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl UpdateSettings {
    pub fn validate(&self) -> Result<()> {
        if let Some(url) = &self.feed_url {
            https_url(url, "update feed address")?;
        }
        if let Some(version) = &self.skipped_version {
            semver::Version::parse(version).map_err(|_| anyhow::anyhow!("The skipped version is not a valid version number"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReleasePackage {
    pub kind: String,
    pub arch: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    #[serde(default)]
    pub published: String,
    pub notes_url: String,
    pub packages: Vec<ReleasePackage>,
}

#[derive(Debug, Clone, Deserialize)]
struct Feed {
    schema: u32,
    channels: BTreeMap<String, Release>,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    payload: String,
    signature: String,
}

fn https_url(text: &str, what: &str) -> Result<()> {
    let url = url::Url::parse(text).map_err(|_| anyhow::anyhow!("The {what} is not a valid address"))?;
    if url.scheme() != "https" || url.host_str().is_none() {
        bail!("The {what} must be an https:// address");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("The {what} must not contain a user name or password");
    }
    Ok(())
}

fn validate_release(release: &Release) -> Result<semver::Version> {
    let version = semver::Version::parse(&release.version).map_err(|_| anyhow::anyhow!("The release has an invalid version number"))?;
    https_url(&release.notes_url, "release notes address")?;
    if release.packages.len() > 20 {
        bail!("The release lists too many packages");
    }
    for package in &release.packages {
        if !PACKAGE_KINDS.contains(&package.kind.as_str()) {
            bail!("The release lists an unknown package type '{}'", package.kind.chars().take(20).collect::<String>());
        }
        https_url(&package.url, "package address")?;
        if package.sha256.len() != 64 || !package.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("A package has no valid SHA-256 checksum");
        }
        if package.size == 0 || package.size > MAX_PACKAGE_BYTES {
            bail!("A package has an invalid size");
        }
        if package.arch.is_empty() || package.arch.len() > 20 || !package.arch.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            bail!("A package has an invalid architecture");
        }
    }
    Ok(version)
}

/// Check the signature, then parse. A feed that fails either step is never shown to the user.
pub fn verify_feed(body: &[u8], public_key: &[u8; 32]) -> Result<BTreeMap<String, Release>> {
    if body.len() > MAX_FEED_BYTES {
        bail!("The update feed is larger than expected and was ignored");
    }
    let envelope: Envelope = serde_json::from_slice(body).context("The update feed is not readable")?;
    let engine = base64::engine::general_purpose::STANDARD;
    let payload = engine.decode(envelope.payload.trim()).map_err(|_| anyhow::anyhow!("The update feed payload is not valid"))?;
    let signature_bytes: [u8; 64] = engine.decode(envelope.signature.trim()).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| anyhow::anyhow!("The update feed has no valid signature"))?;
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| anyhow::anyhow!("The built-in update key is not valid"))?;
    key.verify_strict(&payload, &Signature::from_bytes(&signature_bytes))
        .map_err(|_| anyhow::anyhow!("The update feed's signature is not valid, so it was ignored"))?;
    let feed: Feed = serde_json::from_slice(&payload).context("The update feed has an unexpected format")?;
    if feed.schema != 1 {
        bail!("The update feed uses a newer format than this version understands");
    }
    for release in feed.channels.values() {
        validate_release(release)?;
    }
    Ok(feed.channels)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum UpdateCheck {
    /// This build has no update channel (no signing key or feed address).
    NotConfigured { reason: String },
    UpToDate { current: String },
    Available { release: Release, managed_by_package_manager: bool },
    /// A newer version exists but the user chose to skip it.
    Skipped { version: String },
    NoRelease { channel: UpdateChannel },
}

/// Is there something newer for this channel and machine? Pure.
pub fn evaluate(channels: &BTreeMap<String, Release>, current: &str, settings: &UpdateSettings, arch: &str, managed: bool) -> Result<UpdateCheck> {
    let current_version = semver::Version::parse(current).context("This app's own version is not a valid version number")?;
    // Beta users also see a newer stable release; stable users never see beta.
    let wanted: &[&str] = match settings.channel {
        UpdateChannel::Stable => &["stable"],
        UpdateChannel::Beta => &["beta", "stable"],
    };
    let mut best: Option<(semver::Version, &Release)> = None;
    for key in wanted {
        if let Some(release) = channels.get(*key) {
            let version = validate_release(release)?;
            if best.as_ref().is_none_or(|(b, _)| version > *b) {
                best = Some((version, release));
            }
        }
    }
    let Some((version, release)) = best else {
        return Ok(UpdateCheck::NoRelease { channel: settings.channel });
    };
    if version <= current_version {
        return Ok(UpdateCheck::UpToDate { current: current.to_string() });
    }
    if settings.skipped_version.as_deref() == Some(release.version.as_str()) {
        return Ok(UpdateCheck::Skipped { version: release.version.clone() });
    }
    let mut release = release.clone();
    // Only packages for this machine are offered; others would be misleading.
    release.packages.retain(|p| p.arch == arch);
    Ok(UpdateCheck::Available { release, managed_by_package_manager: managed })
}

/// Whether this app lives in a system folder, which means a package manager owns its files and
/// is the right way to update it.
pub fn installed_by_package_manager(executable: &std::path::Path) -> bool {
    ["/usr/", "/opt/", "/app/"].iter().any(|prefix| executable.starts_with(prefix)) && !executable.starts_with("/usr/local/")
}

/// Fetch the feed over HTTPS with a size and time limit.
pub async fn fetch_feed(url: &str) -> Result<Vec<u8>> {
    https_url(url, "update feed address")?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 3 || attempt.url().scheme() != "https" {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()?;
    let mut response = client.get(url).send().await.context("Could not reach the update server")?.error_for_status().context("The update server refused the request")?;
    if response.content_length().is_some_and(|n| n as usize > MAX_FEED_BYTES) {
        bail!("The update feed is larger than expected and was ignored");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > MAX_FEED_BYTES {
            bail!("The update feed is larger than expected and was ignored");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Run a check with an injected fetcher (tests use a fake feed; the app uses `fetch_feed`).
pub async fn check<F, Fut>(settings: &UpdateSettings, public_key: Option<[u8; 32]>, current: &str, executable: &std::path::Path, fetch: F) -> Result<UpdateCheck>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u8>>>,
{
    let Some(key) = public_key else {
        return Ok(UpdateCheck::NotConfigured { reason: "This build was made without an update signing key, so it cannot check for updates. Update through the place you installed it from.".into() });
    };
    let Some(url) = settings.feed_url.as_deref().filter(|u| !u.trim().is_empty()) else {
        return Ok(UpdateCheck::NotConfigured { reason: "No update channel is configured. Update through the place you installed it from.".into() });
    };
    https_url(url, "update feed address")?;
    let body = fetch(url.to_string()).await?;
    let channels = verify_feed(&body, &key)?;
    evaluate(&channels, current, settings, std::env::consts::ARCH, installed_by_package_manager(executable))
}
