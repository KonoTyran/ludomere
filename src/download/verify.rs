use super::protocol::{download_url, resolve_download_response, response_filename};
use crate::domain::RemoteArtifact;
use anyhow::{Context, Result, bail};
use quick_xml::{Reader, events::Event};
use std::{fs, io::Read, path::Path, time::Duration};

#[derive(Debug)]
pub struct GogChecksum {
    pub filename: String,
    pub md5: String,
    pub size: u64,
}

pub fn gog_checksum(artifact: &RemoteArtifact, access_token: &str) -> Result<GogChecksum> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(crate::identity::USER_AGENT)
        .build()?;
    let response = client
        .get(download_url(&artifact.download_path))
        .bearer_auth(access_token)
        .send()?
        .error_for_status()?;
    let resolved = resolve_download_response(&client, response, None)?;
    let response = resolved.response;
    let fallback_name = response_filename(&response);
    let metadata_url = if let Some(checksum) = resolved.checksum_url {
        checksum
    } else {
        let mut url = response.url().clone();
        url.set_path(&format!("{}.xml", url.path()));
        url.into()
    };
    let xml = client
        .get(metadata_url)
        .send()?
        .error_for_status()?
        .text()?;
    parse_gog_checksum(&xml, fallback_name.as_deref())
}

pub fn file_md5_with_progress(path: &Path, mut progress: impl FnMut(u64, u64)) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let total = file.metadata()?.len();
    let mut context = md5::Context::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut read = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        context.consume(&buffer[..count]);
        read += count as u64;
        progress(read, total);
    }
    Ok(format!("{:x}", context.compute()))
}

fn parse_gog_checksum(xml: &str, fallback_name: Option<&str>) -> Result<GogChecksum> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(element) | Event::Empty(element) if element.name().as_ref() == b"file" => {
                let mut filename = None;
                let mut checksum = None;
                let mut size = None;
                for attribute in element.attributes() {
                    let attribute = attribute?;
                    let value = attribute.unescape_value()?.into_owned();
                    match attribute.key.as_ref() {
                        b"name" => filename = Some(value),
                        b"md5" => checksum = Some(value),
                        b"total_size" => size = value.parse().ok(),
                        _ => {}
                    }
                }
                return Ok(GogChecksum {
                    filename: filename
                        .or_else(|| fallback_name.map(str::to_owned))
                        .context("GOG checksum metadata has no filename")?,
                    md5: checksum.context("GOG checksum metadata has no MD5")?,
                    size: size.context("GOG checksum metadata has no exact size")?,
                });
            }
            Event::Eof => bail!("GOG checksum metadata contains no file record"),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gog_checksum_xml() {
        let checksum = parse_gog_checksum(
            r#"<file name="setup_game.exe" md5="9dd2b837300bfa19c6b5b8fde5d38df6" total_size="550072224"/>"#,
            None,
        )
        .unwrap();
        assert_eq!(checksum.filename, "setup_game.exe");
        assert_eq!(checksum.md5, "9dd2b837300bfa19c6b5b8fde5d38df6");
        assert_eq!(checksum.size, 550_072_224);
    }
}
