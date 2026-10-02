use crate::domain::Screenshot;
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Condvar, LazyLock, Mutex},
};

static REQUESTS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());
static CLIENT: LazyLock<Result<reqwest::blocking::Client>> = LazyLock::new(crate::gog::client);

struct RequestSlot;
impl Drop for RequestSlot {
    fn drop(&mut self) {
        *REQUESTS.0.lock().unwrap_or_else(|error| error.into_inner()) -= 1;
        REQUESTS.1.notify_one();
    }
}

pub fn cached_image(_product_id: i64, screenshot: &Screenshot, full: bool) -> Result<PathBuf> {
    let mut active = REQUESTS.0.lock().unwrap_or_else(|error| error.into_inner());
    while *active >= 4 {
        active = REQUESTS
            .1
            .wait(active)
            .unwrap_or_else(|error| error.into_inner());
    }
    *active += 1;
    drop(active);
    let _slot = RequestSlot;
    let url = if full {
        &screenshot.full_url
    } else {
        &screenshot.thumbnail_url
    };
    let url = crate::online::normalize_asset_url(url);
    let digest = format!("{:x}", Sha256::digest(url.as_bytes()));
    let directory = crate::identity::screenshots()
        .join("by-source")
        .join(digest);
    let path = directory.join("asset.jpg");
    let client = CLIENT
        .as_ref()
        .map_err(|_| anyhow::anyhow!("Could not initialize image downloads"))?;
    crate::online::cache_cover_at(client, &url, &path)
        .map_err(|error| anyhow::anyhow!("{}", crate::online::sync_error_message(&error)))
}
