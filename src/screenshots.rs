use crate::domain::Screenshot;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

pub fn cached_image(_product_id: i64, screenshot: &Screenshot, full: bool) -> Result<PathBuf> {
    let extension = if (if full {
        &screenshot.full_url
    } else {
        &screenshot.thumbnail_url
    })
    .to_ascii_lowercase()
    .contains(".png")
    {
        "png"
    } else {
        "jpg"
    };
    let url = if full {
        &screenshot.full_url
    } else {
        &screenshot.thumbnail_url
    };
    let digest = format!("{:x}", Sha256::digest(url.as_bytes()));
    let directory = crate::identity::screenshots()
        .join("by-source")
        .join(digest);
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("asset.{extension}"));
    if path.is_file() && path.metadata().is_ok_and(|metadata| metadata.len() > 0) {
        return Ok(path);
    }
    let bytes = reqwest::blocking::get(url)
        .with_context(|| format!("requesting screenshot {url}"))?
        .error_for_status()?
        .bytes()?;
    let temporary = path.with_extension(format!("{extension}.part"));
    fs::write(&temporary, &bytes)?;
    fs::rename(&temporary, &path)?;
    Ok(path)
}
