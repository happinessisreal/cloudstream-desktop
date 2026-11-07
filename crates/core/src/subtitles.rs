//! Bounded, in-memory inspection of user-selected subtitle ZIP archives.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{io::{Cursor, Read}, path::Path};

pub const MAX_SUBTITLE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 256;

/// Recognise ZIP resources from the URL path or explicit provider format metadata.
pub fn is_archive_subtitle(subtitle: &crate::models::Subtitle) -> bool {
    let by_url = url::Url::parse(&subtitle.url).ok().and_then(|url| {
        std::path::Path::new(url.path()).extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("zip"))
    }).unwrap_or(false);
    let by_format = ["format", "mimeType", "mime_type"].iter().any(|key| subtitle.extra.get(*key).and_then(|v| v.as_str()).is_some_and(|v| v.eq_ignore_ascii_case("zip") || v.eq_ignore_ascii_case("application/zip")));
    by_url || by_format
}

pub fn supported_subtitle(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|ext|
        ["srt", "vtt", "ass", "ssa", "ttml", "dfxp", "smi", "sami"].iter().any(|known| ext.eq_ignore_ascii_case(known)))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSubtitle {
    pub index: usize,
    pub name: String,
    pub size: u64,
}

pub fn list_archive(bytes: &[u8]) -> Result<Vec<ArchiveSubtitle>> {
    if bytes.len() as u64 > MAX_SUBTITLE_BYTES { bail!("Subtitle archives must be 20 MiB or smaller"); }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context("Could not read this ZIP archive")?;
    if archive.len() > MAX_ARCHIVE_ENTRIES { bail!("Subtitle archives may contain at most {MAX_ARCHIVE_ENTRIES} entries"); }
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).context("Could not inspect an archive entry")?;
        if entry.name().contains('\\') { bail!("The archive contains an unsafe path"); }
        let path = entry.enclosed_name().context("The archive contains an unsafe path")?;
        if entry.is_symlink() { bail!("Subtitle archives cannot contain symbolic links"); }
        if !entry.is_file() || !supported_subtitle(&path) { continue; }
        if entry.size() > MAX_SUBTITLE_BYTES { bail!("A subtitle in the archive exceeds 20 MiB"); }
        let name = path.to_str().context("An archive filename cannot be represented as text")?.to_string();
        entries.push(ArchiveSubtitle { index, name, size: entry.size() });
    }
    if entries.is_empty() { bail!("The ZIP archive contains no supported subtitle files"); }
    Ok(entries)
}

/// Decompress exactly one entry, without using its path for output. Re-inspection ensures callers
/// cannot request an unsupported or unsafe member even if an index was forged.
pub fn extract_archive_entry(bytes: &[u8], index: usize) -> Result<(ArchiveSubtitle, Vec<u8>)> {
    let entry = list_archive(bytes)?.into_iter().find(|entry| entry.index == index)
        .context("That subtitle is no longer available in the archive")?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let file = archive.by_index(index)?;
    let mut output = Vec::new();
    file.take(MAX_SUBTITLE_BYTES + 1).read_to_end(&mut output).context("Could not extract the subtitle")?;
    if output.len() as u64 > MAX_SUBTITLE_BYTES { bail!("The extracted subtitle exceeds 20 MiB"); }
    Ok((entry, output))
}

/// Optional cleanup for a copied subtitle. The source is never modified.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct TextTransforms {
    pub remove_captions: bool,
    pub remove_music: bool,
    pub remove_bloat: bool,
    pub uppercase: bool,
}

impl TextTransforms {
    pub fn is_empty(&self) -> bool {
        !self.remove_captions && !self.remove_music && !self.remove_bloat && !self.uppercase
    }
}

/// Transform supported cue text only, preserving ASS/SSA styles and control sequences.
/// UTF-8 is required so an unknown legacy encoding cannot be corrupted.
pub fn transform_text(bytes: &[u8], extension: &str, options: &TextTransforms) -> Result<Vec<u8>> {
    if bytes.len() as u64 > MAX_SUBTITLE_BYTES { bail!("Subtitle files must be 20 MiB or smaller"); }
    if options.is_empty() { return Ok(bytes.to_vec()); }
    let text = std::str::from_utf8(bytes).context("Subtitle cleanup requires UTF-8 text; the original was not changed")?;
    if extension.eq_ignore_ascii_case("ass") || extension.eq_ignore_ascii_case("ssa") {
        return transform_ass(text, options);
    }
    if !extension.eq_ignore_ascii_case("srt") && !extension.eq_ignore_ascii_case("vtt") {
        bail!("Subtitle cleanup supports SRT, WebVTT, ASS and SSA files; the original was not changed");
    }
    let bom = text.starts_with('\u{feff}');
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let mut output = String::new();
    if bom { output.push('\u{feff}'); }
    let mut cues = 0;
    for block in text.split("\n\n") {
        if block.is_empty() { continue; }
        let lines: Vec<&str> = block.split('\n').collect();
        let first = lines[0].trim_start();
        let metadata = first.starts_with("WEBVTT") || first == "NOTE" || first.strip_prefix("NOTE").is_some_and(|rest| rest.starts_with(char::is_whitespace)) || first == "STYLE" || first == "REGION";
        let timing = if metadata { None } else { lines.iter().position(|line| is_timing_line(line)) };
        let Some(timing) = timing else {
            output.push_str(block);
            output.push_str("\n\n");
            continue;
        };
        cues += 1;
        let mut content = Vec::new();
        for line in &lines[timing + 1..] {
            let cleaned = clean_cue_line(line, options);
            if !cleaned.trim().is_empty() { content.push(cleaned); }
        }
        if content.is_empty() { continue; }
        for line in &lines[..=timing] { output.push_str(line); output.push('\n'); }
        output.push_str(&content.join("\n"));
        output.push_str("\n\n");
        if output.len() as u64 > MAX_SUBTITLE_BYTES { bail!("The cleaned subtitle exceeds 20 MiB"); }
    }
    if cues == 0 { bail!("No subtitle cues were found; the original was not changed"); }
    if output.len() as u64 > MAX_SUBTITLE_BYTES { bail!("The cleaned subtitle exceeds 20 MiB"); }
    Ok(output.into_bytes())
}

fn is_timing_line(line: &str) -> bool {
    let Some((start, end)) = line.split_once("-->") else { return false };
    start.trim().chars().next().is_some_and(|c| c.is_ascii_digit())
        && end.trim().chars().next().is_some_and(|c| c.is_ascii_digit())
        && start.contains(':') && end.contains(':')
}

fn clean_cue_line(line: &str, options: &TextTransforms) -> String {
    // Music markers identify an entire music line, not adjacent dialogue in another line.
    if options.remove_music && line.chars().any(|c| matches!(c, '♪' | '♫' | '♬' | '♩')) { return String::new(); }
    let lower = line.to_lowercase();
    if options.remove_bloat && (lower.contains("http://") || lower.contains("https://") || lower.contains("www.")
        || lower.trim_start().starts_with("subtitles by ") || lower.trim_start().starts_with("translated by ")) {
        return String::new();
    }
    let mut output = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        // Preserve valid markup and character references byte-for-byte, including case.
        if c == '<' {
            let mut look = chars.clone();
            if look.any(|next| next == '>') {
                output.push(c);
                for next in chars.by_ref() { output.push(next); if next == '>' { break; } }
                continue;
            }
            // An unclosed tag is ambiguous; retain the remainder rather than corrupt it.
            output.push(c);
            output.extend(chars);
            break;
        }
        if c == '&' && chars.clone().take(32).any(|next| next == ';') {
            output.push(c);
            for next in chars.by_ref() { output.push(next); if next == ';' { break; } }
            continue;
        }
        // Square brackets conventionally mark closed-caption descriptions; parentheses are
        // deliberately retained because they often contain spoken dialogue.
        if options.remove_captions && c == '[' {
            let mut look = chars.clone();
            if look.any(|next| next == ']') {
                for next in chars.by_ref() { if next == ']' { break; } }
                continue;
            }
            output.push(c);
            output.extend(chars);
            break;
        }
        if options.uppercase { output.extend(c.to_uppercase()); } else { output.push(c); }
    }
    output.trim().to_string()
}

/// Identify supported text formats from the original provider URL or explicit format metadata.
pub fn text_format(subtitle: &crate::models::Subtitle) -> Option<&'static str> {
    let from_url = url::Url::parse(&subtitle.url).ok().and_then(|url| {
        Path::new(url.path()).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase)
    });
    let classify = |s: &str| match s.split(';').next().unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "srt" | "application/x-subrip" | "application/srt" => Some("srt"),
        "vtt" | "webvtt" | "text/vtt" => Some("vtt"),
        "ass" | "text/x-ssa" | "application/x-ass" => Some("ass"),
        "ssa" | "application/x-ssa" => Some("ssa"),
        _ => None,
    };
    from_url.as_deref().and_then(classify).or_else(|| ["format", "mimeType", "mime_type"].iter()
        .filter_map(|key| subtitle.extra.get(*key).and_then(|value| value.as_str()))
        .find_map(classify))
}

/// Read a subtitle through its own resource session with a strict size and time limit.
pub async fn fetch_for_cleanup(url: &str, no_proxy: bool) -> Result<Vec<u8>> {
    crate::repository::network_url(url)?;
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 || crate::repository::network_url(attempt.url().as_str()).is_err() {
            attempt.error("Invalid subtitle redirect")
        } else { attempt.follow() }
    });
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).redirect(policy);
    if no_proxy { builder = builder.no_proxy(); }
    let mut response = builder.build()?.get(url).send().await.context("Could not download subtitle")?
        .error_for_status().context("The provider rejected the subtitle request")?;
    if response.content_length().is_some_and(|size| size > MAX_SUBTITLE_BYTES) { bail!("Subtitle files must be 20 MiB or smaller"); }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.context("Could not read subtitle")? {
        if bytes.len().saturating_add(chunk.len()) > MAX_SUBTITLE_BYTES as usize { bail!("Subtitle files must be 20 MiB or smaller"); }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Preserve all ASS headers and event fields; only the Text field in Dialogue events changes.
/// Text must be the last declared field because dialogue text may itself contain commas.
fn transform_ass(text: &str, options: &TextTransforms) -> Result<Vec<u8>> {
    let mut output = String::with_capacity(text.len());
    let mut in_events = false;
    let mut fields = None;
    let mut dialogues = 0usize;
    for raw in text.split_inclusive('\n') {
        let (line, ending) = if let Some(line) = raw.strip_suffix("\r\n") { (line, "\r\n") }
            else if let Some(line) = raw.strip_suffix('\n') { (line, "\n") }
            else { (raw, "") };
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_events = trimmed.eq_ignore_ascii_case("[Events]");
            if in_events { fields = None; }
        }
        let Some((kind, rest)) = line.split_once(':') else {
            output.push_str(line); output.push_str(ending); continue;
        };
        if in_events && kind.trim().eq_ignore_ascii_case("format") {
            let parts: Vec<_> = rest.split(',').map(str::trim).collect();
            if parts.len() < 2 || !parts.last().is_some_and(|p| p.eq_ignore_ascii_case("text"))
                || parts[..parts.len()-1].iter().any(|p| p.eq_ignore_ascii_case("text")) {
                bail!("ASS subtitle event format is unsupported; the original was not changed");
            }
            fields = Some(parts.len());
        }
        if in_events && kind.trim().eq_ignore_ascii_case("dialogue") {
            let count = fields.context("ASS subtitle has no event format; the original was not changed")?;
            let mut offset = 0;
            for _ in 1..count {
                let comma = rest[offset..].find(',').context("ASS subtitle dialogue has missing fields; the original was not changed")?;
                offset += comma + 1;
            }
            output.push_str(kind); output.push(':'); output.push_str(&rest[..offset]);
            output.push_str(&clean_ass_text(&rest[offset..], options)?);
            output.push_str(ending);
            dialogues += 1;
        } else {
            output.push_str(line); output.push_str(ending);
        }
        if output.len() as u64 > MAX_SUBTITLE_BYTES { bail!("The cleaned subtitle exceeds 20 MiB"); }
    }
    if dialogues == 0 { bail!("No ASS subtitle dialogue was found; the original was not changed"); }
    if output.len() as u64 > MAX_SUBTITLE_BYTES { bail!("The cleaned subtitle exceeds 20 MiB"); }
    Ok(output.into_bytes())
}

/// A logical ASS line is divided into text and protected tokens (override tags, escapes and
/// vector drawing commands). Only plain text tokens are subject to cleanup or case changes.
fn clean_ass_text(text: &str, options: &TextTransforms) -> Result<String> {
    fn flush(tokens: &mut Vec<(bool, String)>, output: &mut String, options: &TextTransforms) {
        let visible = tokens.iter().filter(|(plain, _)| *plain).map(|(_, s)| s.as_str()).collect::<String>();
        let lower = visible.to_lowercase();
        let drop_text = (options.remove_music && visible.chars().any(|c| matches!(c, '♪' | '♫' | '♬' | '♩')))
            || (options.remove_bloat && (lower.contains("http://") || lower.contains("https://") || lower.contains("www.")
                || lower.trim_start().starts_with("subtitles by ") || lower.trim_start().starts_with("translated by ")));
        let mut visible_offset = 0;
        let mut inside_caption = false;
        let mut may_close_caption = options.remove_captions;
        for (plain, value) in tokens.drain(..) {
            if !plain { output.push_str(&value); continue; }
            if drop_text { continue; }
            for c in value.chars() {
                visible_offset += c.len_utf8();
                if inside_caption {
                    if c == ']' { inside_caption = false; }
                    continue;
                }
                if may_close_caption && c == '[' {
                    if visible[visible_offset..].contains(']') { inside_caption = true; continue; }
                    may_close_caption = false;
                }
                if options.uppercase { output.extend(c.to_uppercase()); } else { output.push(c); }
            }
        }
    }
    let mut out = String::new();
    let mut tokens: Vec<(bool, String)> = Vec::new();
    let mut drawing = false;
    let mut position = 0;
    while position < text.len() {
        let rest = &text[position..];
        if rest.starts_with('{') {
            let end = rest.find('}').context("ASS subtitle has an unclosed override tag; the original was not changed")? + 1;
            let tag = &rest[..end];
            for command in tag[1..tag.len()-1].split('\\').skip(1) {
                if let Some(value) = command.strip_prefix('p') {
                    let digits: String = value.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if !digits.is_empty() { drawing = digits.parse::<u32>().unwrap_or(1) != 0; }
                }
            }
            tokens.push((false, tag.to_string())); position += end;
        } else if rest.starts_with('\\') && rest.len() >= 2 && matches!(rest.as_bytes()[1], b'N' | b'n' | b'h') {
            let escape = &rest[..2];
            if escape != "\\h" { flush(&mut tokens, &mut out, options); out.push_str(escape); }
            else { tokens.push((false, escape.to_string())); }
            position += 2;
        } else {
            let ch = rest.chars().next().unwrap();
            if let Some((plain, value)) = tokens.last_mut() {
                if *plain == !drawing { value.push(ch); position += ch.len_utf8(); continue; }
            }
            tokens.push((!drawing, ch.to_string()));
            position += ch.len_utf8();
        }
    }
    flush(&mut tokens, &mut out, options);
    Ok(out)
}
