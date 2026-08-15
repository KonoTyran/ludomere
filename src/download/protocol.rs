use super::files::sanitize_filename;
use anyhow::{Context, Result, bail};
use reqwest::{
    blocking::{Client, Response},
    header,
};

#[derive(Debug, serde::Deserialize)]
struct GogDownlinkDescriptor {
    downlink: String,
    checksum: Option<String>,
}

pub(super) struct ResolvedDownload {
    pub response: Response,
    pub checksum_url: Option<String>,
}

pub(super) fn resolve_download_response(
    client: &Client,
    response: Response,
    range_start: Option<u64>,
) -> Result<ResolvedDownload> {
    if !is_json_response(&response) {
        return Ok(ResolvedDownload {
            response,
            checksum_url: None,
        });
    }
    let descriptor = response
        .json::<GogDownlinkDescriptor>()
        .context("GOG returned JSON instead of downloadable content")?;
    if descriptor.downlink.trim().is_empty() {
        bail!("GOG download descriptor contains no download URL");
    }
    let mut request = client.get(&descriptor.downlink);
    if let Some(start) = range_start.filter(|start| *start > 0) {
        request = request.header(header::RANGE, format!("bytes={start}-"));
    }
    let response = request
        .send()
        .context("requesting the signed GOG download URL")?
        .error_for_status()
        .context("GOG CDN rejected the signed download URL")?;
    if is_json_response(&response) {
        bail!("GOG CDN returned JSON instead of downloadable content");
    }
    Ok(ResolvedDownload {
        response,
        checksum_url: descriptor.checksum,
    })
}

fn is_json_response(response: &Response) -> bool {
    let content_type_is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
        });
    // Some GOG downlink responses omit or misreport Content-Type. These API
    // endpoints return a small JSON descriptor, never the installer payload.
    content_type_is_json
        || (response
            .url()
            .host_str()
            .is_some_and(|host| host == "gog.com" || host.ends_with(".gog.com"))
            && response.url().path().contains("/downlink/")
            && response
                .content_length()
                .is_none_or(|size| size <= 64 * 1024))
}

pub(super) fn response_filename(response: &Response) -> Option<String> {
    let from_disposition = response
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .and_then(content_disposition_filename);
    from_disposition.or_else(|| {
        let segment = response.url().path_segments()?.next_back()?;
        let filename = percent_decode(segment)?;
        filename
            .contains('.')
            .then(|| sanitize_filename(&filename))
            .flatten()
    })
}

pub(super) fn content_disposition_filename(disposition: &str) -> Option<String> {
    let parts = disposition.split(';').map(str::trim).collect::<Vec<_>>();
    if let Some(encoded) = parts.iter().find_map(|part| {
        part.strip_prefix("filename*=")
            .or_else(|| part.strip_prefix("Filename*="))
    }) {
        let encoded = encoded
            .split_once("''")
            .map_or(encoded, |(_, value)| value)
            .trim_matches('"');
        if let Some(decoded) = percent_decode(encoded).and_then(|value| sanitize_filename(&value)) {
            return Some(decoded);
        }
    }
    parts
        .iter()
        .find_map(|part| {
            part.strip_prefix("filename=")
                .or_else(|| part.strip_prefix("Filename="))
        })
        .and_then(|value| sanitize_filename(value.trim_matches('"')))
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3)?;
            let text = std::str::from_utf8(hex).ok()?;
            decoded.push(u8::from_str_radix(text, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

pub(super) fn download_url(path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        path.to_owned()
    } else {
        format!(
            "https://www.gog.com{}",
            if path.starts_with('/') {
                path.into()
            } else {
                format!("/{path}")
            }
        )
    }
}
