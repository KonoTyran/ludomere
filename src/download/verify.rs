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
        .timeout(Duration::from_secs(45))
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
    let response = client.get(metadata_url).send()?.error_for_status()?;
    let mut xml = String::new();
    response.take(1024 * 1024 + 1).read_to_string(&mut xml)?;
    anyhow::ensure!(
        xml.len() <= 1024 * 1024,
        "GOG checksum metadata exceeded its size limit"
    );
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
                let filename = filename
                    .or_else(|| fallback_name.map(str::to_owned))
                    .context("GOG checksum metadata has no filename")?;
                anyhow::ensure!(
                    !filename.trim().is_empty()
                        && !filename
                            .chars()
                            .any(|character| character.is_control()
                                || matches!(character, '/' | '\\'))
                        && matches!(
                            Path::new(&filename).components().next(),
                            Some(std::path::Component::Normal(_))
                        )
                        && Path::new(&filename).components().count() == 1,
                    "GOG checksum metadata filename must be a single safe filename"
                );
                let md5 = checksum.context("GOG checksum metadata has no MD5")?;
                anyhow::ensure!(
                    md5.len() == 32 && md5.bytes().all(|byte| byte.is_ascii_hexdigit()),
                    "GOG checksum metadata MD5 must contain exactly 32 hexadecimal digits"
                );
                return Ok(GogChecksum {
                    filename,
                    md5,
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

    #[test]
    fn checksum_names_reject_paths_without_normalizing_or_using_fallback() {
        for filename in [
            "",
            " ",
            ".",
            "..",
            "../outside.exe",
            "/outside.exe",
            "sub/setup.exe",
            "./setup.exe",
            "setup.exe/",
            "C:\\outside.exe",
            "..\\outside.exe",
            "\\\\host\\setup.exe",
            "bad\0.exe",
            "bad\n.exe",
        ] {
            let xml = format!(
                "<file name=\"{filename}\" md5=\"9dd2b837300bfa19c6b5b8fde5d38df6\" total_size=\"1\"/>"
            );
            assert!(
                parse_gog_checksum(&xml, Some("safe_fallback.exe")).is_err(),
                "unsafe XML name {filename:?}"
            );
            assert!(
                parse_gog_checksum(
                    "<file md5=\"9dd2b837300bfa19c6b5b8fde5d38df6\" total_size=\"1\"/>",
                    Some(filename)
                )
                .is_err(),
                "unsafe fallback name {filename:?}"
            );
        }
        for filename in [
            "setup_game.exe",
            "setup_game-1.bin",
            "soundtrack bonus.zip",
            "日本語の特典.zip",
            " leading space.zip",
            "trailing space.zip ",
        ] {
            let xml = format!(
                "<file name=\"{filename}\" md5=\"9dd2b837300bfa19c6b5b8fde5d38df6\" total_size=\"1\"/>"
            );
            assert_eq!(
                parse_gog_checksum(&xml, Some("other.exe"))
                    .unwrap()
                    .filename,
                filename
            );
            assert_eq!(
                parse_gog_checksum(
                    "<file md5=\"9dd2b837300bfa19c6b5b8fde5d38df6\" total_size=\"1\"/>",
                    Some(filename)
                )
                .unwrap()
                .filename,
                filename
            );
        }
        assert!(
            parse_gog_checksum(
                "<file name=\"..&#47;outside.exe\" md5=\"9dd2b837300bfa19c6b5b8fde5d38df6\" total_size=\"1\"/>",
                None
            )
            .is_err()
        );
        assert!(
            parse_gog_checksum(
                "<file name=\"bad&#10;.exe\" md5=\"9dd2b837300bfa19c6b5b8fde5d38df6\" total_size=\"1\"/>",
                None
            )
            .is_err()
        );
    }

    #[test]
    fn checksum_hashes_reject_malformed_values_and_preserve_hex_case() {
        for hash in [
            "",
            "abc",
            "9dd2b837300bfa19c6b5b8fde5d38df",
            "9dd2b837300bfa19c6b5b8fde5d38df60",
            "zdd2b837300bfa19c6b5b8fde5d38df6",
            " dd2b837300bfa19c6b5b8fde5d38df6",
            "9dd2b837300bfa19c6b5b8fde5d38df6 ",
        ] {
            let xml = format!("<file name=\"setup.exe\" md5=\"{hash}\" total_size=\"1\"/>");
            assert!(
                parse_gog_checksum(&xml, None)
                    .unwrap_err()
                    .to_string()
                    .contains("MD5")
            );
        }
        for hash in [
            "9dd2b837300bfa19c6b5b8fde5d38df6",
            "9DD2B837300BFA19C6B5B8FDE5D38DF6",
        ] {
            let xml = format!("<file name=\"setup.exe\" md5=\"{hash}\" total_size=\"1\"/>");
            assert_eq!(parse_gog_checksum(&xml, None).unwrap().md5, hash);
        }
    }
}
