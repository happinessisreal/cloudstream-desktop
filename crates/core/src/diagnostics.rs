//! Diagnostics (T32): what went wrong and how long things took, without secrets.
//!
//! Two small rings live in memory: recent events (errors and notices) and recent request
//! timings. Both are bounded in count and in text length, so they cannot grow without limit, and
//! nothing is written to disk until the user exports a report. Every free-text message passes
//! through `redact` before it is stored. Structured facts (versions, package hashes, capability
//! failures) are read directly from the app and never pass through log text.
use crate::models::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, sync::{Mutex, OnceLock}};

pub const MAX_EVENTS: usize = 500;
pub const MAX_TIMINGS: usize = 500;
pub const MAX_MESSAGE_CHARS: usize = 400;
pub const MAX_REPORT_BYTES: usize = 1024 * 1024;

fn patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let rules: [(&str, &str); 7] = [
            // Header lines: the whole value goes.
            (r"(?i)\b(authorization|proxy-authorization|cookie|set-cookie|x-[a-z0-9-]*(?:token|key|secret|auth|session)[a-z0-9-]*)\s*[:=]\s*[^\r\n]*", "$1: [redacted]"),
            // JSON and key=value secrets.
            (r#"(?i)(^|[^A-Za-z0-9_])("?(?:access_?token|refresh_?token|id_?token|token|password|passwd|secret|client_?secret|api_?key|apikey|pin|session(?:_?id)?|cookie|license_?url|key|signature|sig)"?\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,;&}\])]+)"#, "${1}${2}[redacted]"),
            (r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}", "$1 [redacted]"),
            // Email addresses.
            (r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}", "[email]"),
            // Magnet links carry the identity of what is being watched.
            (r"(?i)magnet:\?[^\s]*", "magnet:[redacted]"),
            // Long opaque strings: tokens, keys and session identifiers.
            (r"\b[A-Za-z0-9_\-+/=]{32,}\b", "[redacted]"),
            // Credentials embedded before an at-sign in a URL authority.
            (r"(?i)(https?://)[^/\s@]+@", "$1"),
        ];
        rules.iter().filter_map(|(pattern, replacement)| Regex::new(pattern).ok().map(|r| (r, *replacement))).collect()
    })
}

fn url_pattern() -> &'static Regex {
    static URL: OnceLock<Regex> = OnceLock::new();
    URL.get_or_init(|| Regex::new(r#"(?i)https?://[^\s"'<>)\]}]+"#).expect("valid pattern"))
}

/// Keep where an address points, drop what identifies the user: credentials, every query value
/// and the fragment, plus long path segments that look like tokens.
fn redact_url(url: &str) -> String {
    let without_fragment = url.split('#').next().unwrap_or(url);
    let (base, has_query) = match without_fragment.split_once('?') {
        Some((base, _)) => (base, true),
        None => (without_fragment, false),
    };
    let (scheme, rest) = base.split_once("://").unwrap_or(("https", base));
    let rest = rest.rsplit_once('@').map_or(rest, |(_, after)| after);
    let mut parts = rest.split('/');
    let host = parts.next().unwrap_or("");
    let path: Vec<String> = parts
        .map(|segment| if segment.len() >= 24 && segment.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '=' | '%')) { "[redacted]".to_string() } else { segment.to_string() })
        .collect();
    let mut out = format!("{scheme}://{host}");
    if !path.is_empty() {
        out.push('/');
        out.push_str(&path.join("/"));
    }
    if has_query {
        out.push_str("?[redacted]");
    }
    out
}

/// Remove secrets from free text. Conservative on purpose: when in doubt, it removes.
pub fn redact(text: &str) -> String {
    // Addresses first, so their queries are removed whole before the generic rules run.
    let mut out = url_pattern().replace_all(text, |caps: &regex::Captures| redact_url(&caps[0])).into_owned();
    for (pattern, replacement) in patterns() {
        out = pattern.replace_all(&out, *replacement).into_owned();
    }
    if let Some(home) = std::env::var_os("HOME").map(|h| h.to_string_lossy().into_owned()).filter(|h| h.len() > 1) {
        out = out.replace(&home, "~");
    }
    out
}

fn bounded(text: &str) -> String {
    let cleaned: String = redact(text).chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).collect();
    let single_line = cleaned.replace('\n', " ⏎ ");
    if single_line.chars().count() > MAX_MESSAGE_CHARS {
        let mut cut: String = single_line.chars().take(MAX_MESSAGE_CHARS).collect();
        cut.push('…');
        cut
    } else {
        single_line
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticEvent {
    pub at_ms: u64,
    pub category: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestTiming {
    pub at_ms: u64,
    pub method: String,
    pub duration_ms: u64,
    pub ok: bool,
    /// A short class, never the provider's message: `timeout`, `cancelled` or `error`.
    pub outcome: String,
}

#[derive(Default)]
pub struct Diagnostics {
    events: Mutex<VecDeque<DiagnosticEvent>>,
    timings: Mutex<VecDeque<RequestTiming>>,
}

impl Diagnostics {
    pub fn record(&self, category: &str, message: &str) {
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        if events.len() >= MAX_EVENTS {
            events.pop_front();
        }
        events.push_back(DiagnosticEvent { at_ms: timestamp_ms(), category: category.chars().take(32).collect(), message: bounded(message) });
    }

    pub fn record_timing(&self, method: &str, duration_ms: u64, outcome: &str) {
        let mut timings = self.timings.lock().unwrap_or_else(|e| e.into_inner());
        if timings.len() >= MAX_TIMINGS {
            timings.pop_front();
        }
        let method: String = method.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(48).collect();
        timings.push_back(RequestTiming { at_ms: timestamp_ms(), method, duration_ms, ok: outcome == "ok", outcome: outcome.chars().take(16).collect() });
    }

    pub fn events(&self) -> Vec<DiagnosticEvent> {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
    }

    pub fn timings(&self) -> Vec<RequestTiming> {
        self.timings.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
    }

    pub fn clear(&self) {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.timings.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// A coarse, message-free class for an error, so timings never carry provider text.
pub fn classify_error(error: &anyhow::Error) -> &'static str {
    let text = format!("{error:#}").to_lowercase();
    if text.contains("timed out") || text.contains("timeout") {
        "timeout"
    } else if text.contains("cancel") {
        "cancelled"
    } else {
        "error"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageInfo {
    pub id: String,
    pub name: String,
    pub version: i64,
    pub runtime: String,
    pub sha256: String,
    pub verified: bool,
    /// The repository address with credentials, query and token-like segments removed.
    pub repository: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsReport {
    pub generated_at_ms: u64,
    pub app_version: String,
    pub os: String,
    pub arch: String,
    pub java_available: bool,
    pub worker_available: bool,
    pub worker_jar_sha256: Option<String>,
    pub mpv_version: Option<String>,
    /// Player features this mpv lacks, with the reason.
    pub player_unsupported: Vec<String>,
    pub packages: Vec<PackageInfo>,
    pub providers: usize,
    pub disabled_providers: usize,
    pub events: Vec<DiagnosticEvent>,
    pub timings: Vec<RequestTiming>,
    pub notes: Vec<String>,
}

impl DiagnosticsReport {
    /// The report as text for saving. Bounded, so a runaway log can never produce a huge file.
    pub fn to_json(&self) -> anyhow::Result<Vec<u8>> {
        let mut report = self.clone();
        loop {
            let bytes = serde_json::to_vec_pretty(&report)?;
            if bytes.len() <= MAX_REPORT_BYTES {
                return Ok(bytes);
            }
            // Drop the oldest entries until it fits.
            let drop_events = report.events.len() / 4 + 1;
            report.events.drain(..drop_events.min(report.events.len()));
            let drop_timings = report.timings.len() / 4 + 1;
            report.timings.drain(..drop_timings.min(report.timings.len()));
            if report.events.is_empty() && report.timings.is_empty() {
                return Ok(serde_json::to_vec_pretty(&report)?);
            }
        }
    }
}

pub fn package_info(plugin: &InstalledPlugin) -> PackageInfo {
    PackageInfo {
        id: plugin.id.clone(),
        name: bounded(&plugin.entry.name),
        version: plugin.entry.version as i64,
        runtime: format!("{:?}", plugin.runtime).to_lowercase(),
        sha256: plugin.sha256.clone(),
        verified: plugin.verified,
        repository: redact_url(&plugin.repository_url),
    }
}
