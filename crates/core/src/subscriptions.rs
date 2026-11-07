//! Stable, profile-scoped observations used by the new-episode checker.
use crate::models::{digest_string, DubStatus, Episode, MediaDetails};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionSnapshot {
    pub generation: String,
    pub observed: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    pub pending: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    pub unread: BTreeMap<String, BTreeSet<String>>,
    pub checked_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionChanges {
    pub first_check: bool,
    pub new_episodes: BTreeMap<String, Vec<String>>,
}

/// Derive stable keys without relying on response order. Duplicate data across the flattened
/// list and dub groups is not counted twice. Empty/incomplete responses never reset a baseline.
pub fn episode_snapshot(details: &MediaDetails) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    fn insert(groups: &mut BTreeMap<String, BTreeSet<String>>, group: &str, episode: &Episode) {
        if episode.data.trim().is_empty() { return; }
        let key = if let Some(number) = episode.episode {
            format!("season:{}:episode:{number}", episode.season.unwrap_or(1))
        } else if let Some(id) = episode.id.as_deref().filter(|s| !s.trim().is_empty()) {
            format!("id:{}", digest_string(id))
        } else if !episode.name.trim().is_empty() && !episode.name.trim().eq_ignore_ascii_case("episode") {
            format!("season:{}:name:{}", episode.season.unwrap_or(1), digest_string(&episode.name.trim().to_lowercase()))
        } else {
            // Signed URLs often rotate query tokens; the URL path still identifies the episode.
            let identity = match url::Url::parse(&episode.data) {
                Ok(mut url) if matches!(url.scheme(), "http" | "https") => {
                    let mut stable: Vec<(String, String)> = url.query_pairs().filter_map(|(key, value)| {
                        let key_lower = key.to_ascii_lowercase();
                        if matches!(key_lower.as_str(), "token" | "signature" | "sig" | "expires" | "expiry" | "exp" | "auth" | "authorization" | "key") || key_lower.starts_with("x-amz-") {
                            None
                        } else { Some((key.into_owned(), value.into_owned())) }
                    }).collect();
                    stable.sort();
                    url.set_query(None);
                    if !stable.is_empty() { url.query_pairs_mut().extend_pairs(stable); }
                    url.set_fragment(None); url.to_string()
                },
                _ => episode.data.clone(),
            };
            format!("data:{}", digest_string(&identity))
        };
        groups.entry(group.to_string()).or_default().insert(key);
    }
    if !details.dub_groups.is_empty() {
        for (group, episodes) in &details.dub_groups {
            let normalized = match DubStatus::parse(group) {
                DubStatus::None => group.trim().to_lowercase(),
                status => status.as_str().to_string(),
            };
            for episode in episodes { insert(&mut groups, &normalized, episode); }
        }
    } else if !details.episodes.is_empty() {
        for episode in &details.episodes {
            insert(&mut groups, episode.dub_status.unwrap_or(DubStatus::None).as_str(), episode);
        }
    } else {
        for season in &details.seasons {
            for episode in &season.episodes {
                let mut episode = episode.clone();
                if episode.season.is_none() { episode.season = season.season; }
                insert(&mut groups, episode.dub_status.unwrap_or(DubStatus::None).as_str(), &episode);
            }
        }
    }
    if groups.is_empty() { bail!("The provider returned no available episodes; the previous subscription snapshot was kept"); }
    Ok(groups)
}

/// Summary of a bounded check; provider errors never erase a previous observation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionCheckReport {
    pub checked: usize,
    pub skipped: usize,
    pub failed: usize,
    pub updates: Vec<SubscriptionUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionUpdate {
    pub media_id: String,
    pub title: String,
    pub episode_count: usize,
    pub generation: String,
    pub episodes: BTreeMap<String, BTreeSet<String>>,
}

pub const CHECK_INTERVAL_MS: u64 = 6 * 60 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionAlert {
    pub profile_id: String,
    pub media_id: String,
    pub generation: String,
    pub title: String,
    pub episodes: BTreeMap<String, BTreeSet<String>>,
}

impl SubscriptionAlert {
    pub fn episode_count(&self) -> usize { self.episodes.values().map(BTreeSet::len).sum() }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AlertDeliveryReport { pub delivered: usize, pub failed: usize }

/// Clock-independent scheduling rule, including recovery from a backwards system clock.
pub fn check_is_due(snapshot: Option<&SubscriptionSnapshot>, generation: &str, now_ms: u64) -> bool {
    match snapshot {
        Some(snapshot) if snapshot.generation == generation && snapshot.checked_at_ms <= now_ms =>
            now_ms - snapshot.checked_at_ms >= CHECK_INTERVAL_MS,
        _ => true,
    }
}
