//! Import a user-selected font into an app-owned, content-addressed directory.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{io::Read, path::{Path, PathBuf}};

pub const MAX_FONT_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedFont { pub id: String, pub family: String }

pub fn valid_font_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn file_for(root: &Path, id: &str) -> Result<PathBuf> {
    if !valid_font_id(id) { bail!("Invalid imported font identifier"); }
    let dir = root.join(id);
    if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        bail!("The selected custom font is no longer available. Choose it again");
    }
    for name in ["font.ttf", "font.otf"] {
        let path = dir.join(name);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink()) { return Ok(path); }
    }
    bail!("The selected custom font is no longer available. Choose it again")
}

pub fn read_font(root: &Path, id: &str) -> Result<Vec<u8>> {
    let path = file_for(root, id)?;
    let file = std::fs::File::open(path).context("Could not open the imported font")?;
    let mut bytes = Vec::new();
    file.take(MAX_FONT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FONT_BYTES || crate::repository::digest(&bytes) != id {
        bail!("The imported font is damaged. Choose it again");
    }
    Ok(bytes)
}

pub fn font_directory(root: &Path, id: &str) -> Result<PathBuf> {
    // Validate the cached content as well as the name before giving the directory to mpv.
    read_font(root, id)?;
    Ok(root.join(id))
}

pub fn import_font(root: &Path, source: &Path) -> Result<ImportedFont> {
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !ext.eq_ignore_ascii_case("ttf") && !ext.eq_ignore_ascii_case("otf") { bail!("Choose a TTF or OTF font file"); }
    let file = std::fs::File::open(source).context("Could not open the selected font")?;
    if !file.metadata()?.is_file() { bail!("Choose a font file, not a folder"); }
    let mut bytes = Vec::new();
    file.take(MAX_FONT_BYTES + 1).read_to_end(&mut bytes).context("Could not read the font")?;
    if bytes.len() as u64 > MAX_FONT_BYTES { bail!("Font files must be 10 MiB or smaller"); }
    let suffix = if bytes.starts_with(b"OTTO") { "otf" } else if bytes.starts_with(b"\0\x01\0\0") || bytes.starts_with(b"true") { "ttf" } else { bail!("This is not a supported TTF or OTF font"); };
    let face = ttf_parser::Face::parse(&bytes, 0).map_err(|_| anyhow::anyhow!("This font file is invalid or unsupported"))?;
    let names: Vec<_> = face.names().into_iter().filter(|n| n.name_id == ttf_parser::name_id::FAMILY).collect();
    let family = names.iter().filter(|n| n.language_id == 0x0409).chain(names.iter())
        .find_map(|n| n.to_string().filter(|name| !name.trim().is_empty()))
        .context("This font does not provide a readable family name")?;
    let mut style = crate::models::SubtitleStyle::default();
    style.font = family.clone();
    style.validate().context("The font family name is not supported")?;
    let id = crate::repository::digest(&bytes);
    std::fs::create_dir_all(root).context("Could not create the subtitle font folder")?;
    let final_dir = root.join(&id);
    if final_dir.exists() {
        read_font(root, &id)?;
    } else {
        let temp = root.join(format!(".import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temp).context("Could not create a temporary font folder")?;
        let result = (|| -> Result<()> {
            std::fs::write(temp.join(format!("font.{suffix}")), &bytes)?;
            match std::fs::rename(&temp, &final_dir) {
                Ok(()) => Ok(()),
                Err(_) if final_dir.exists() => { read_font(root, &id)?; Ok(()) },
                Err(error) => Err(error.into()),
            }
        })();
        if temp.exists() { let _ = std::fs::remove_dir_all(&temp); }
        result.context("Could not save the selected font")?;
    }
    Ok(ImportedFont { id, family })
}
