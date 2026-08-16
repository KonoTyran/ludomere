use crate::gog::types::GenerationTwoRepository;
use anyhow::{Context, Result, bail};
use flate2::read::ZlibDecoder;
use std::io::{Cursor, Read};

pub const MAX_EXPANDED_METADATA_BYTES: usize = 64 * 1024 * 1024;

pub fn parse(bytes: &[u8]) -> Result<GenerationTwoRepository> {
    let mut expanded = Vec::new();
    if bytes.starts_with(&[0x78]) {
        ZlibDecoder::new(Cursor::new(bytes))
            .take(MAX_EXPANDED_METADATA_BYTES as u64 + 1)
            .read_to_end(&mut expanded)
            .context("decompressing generation-2 repository")?;
    } else {
        if bytes.len() > MAX_EXPANDED_METADATA_BYTES {
            bail!("expanded repository exceeds 64 MiB");
        }
        expanded.extend_from_slice(bytes);
    }
    if expanded.len() > MAX_EXPANDED_METADATA_BYTES {
        bail!("expanded repository exceeds 64 MiB");
    }

    let repository: GenerationTwoRepository =
        serde_json::from_slice(&expanded).context("parsing generation-2 repository JSON")?;
    validate(&repository)?;
    Ok(repository)
}

fn validate(repository: &GenerationTwoRepository) -> Result<()> {
    if repository.generation != 2 {
        bail!(
            "unsupported repository generation {}",
            repository.generation
        );
    }
    if repository.root_product_id.trim().is_empty() {
        bail!("root product ID is missing");
    }
    if repository.products.is_empty() || repository.depots.is_empty() {
        bail!("repository contains no products");
    }
    if repository
        .products
        .iter()
        .any(|product| product.product_id.trim().is_empty())
    {
        bail!("repository product ID is missing");
    }
    if !repository
        .products
        .iter()
        .any(|product| product.product_id == repository.root_product_id)
    {
        bail!("root product is not present in repository products");
    }
    if repository
        .depots
        .iter()
        .any(|depot| depot.manifest_id.trim().is_empty() || depot.product_id.trim().is_empty())
    {
        bail!("repository depot identity is missing");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;

    const FIXTURE: &str = r#"{
        "version":2, "baseProductId":"100", "buildId":"build-1",
        "platform":"windows", "installDirectory":"Game",
        "products":[
          {"productId":"100","name":"Game","script":"goggame-100.script","temp_arguments":"","temp_executable":"setup.exe",
           "playTasks":[{"type":"launch","name":"Play","executable":"game.exe","arguments":["-windowed"],"workingDirectory":"bin","primary":true}],
           "supportTasks":[{"type":"file","source":"a","destination":"b"}]},
          {"productId":"200","name":"DLC"}
        ],
        "dependencies":["DirectX","MSVC2017"],
        "depots":[
          {"manifest":"0123456789abcdef0123456789abcdef","productId":"100","languages":["en-US"],"osBitness":["64"],"compressedSize":8,"size":10},
          {"manifest":"abcdef0123456789abcdef0123456789","productId":"200","languages":["en-US"],"osBitness":["64"],"compressedSize":4,"size":5,"isGogDepot":true}
        ], "ignoredFutureField":true
    }"#;

    #[test]
    fn parses_raw_repository_metadata() {
        let repository = parse(FIXTURE.as_bytes()).unwrap();
        assert_eq!(repository.products.len(), 2);
        assert_eq!(repository.depots[1].product_id, "200");
        assert_eq!(repository.depots[0].languages, ["en-US"]);
        assert_eq!(
            repository.depots[0].os_bitness.as_deref(),
            Some(["64".to_owned()].as_slice())
        );
        assert_eq!(repository.dependencies, ["DirectX", "MSVC2017"]);
        assert_eq!(
            repository.products[0].play_tasks[0].executable.as_deref(),
            Some("game.exe")
        );
        assert_eq!(
            repository.products[0].play_tasks[0].properties["primary"],
            true
        );
        assert_eq!(
            repository.products[0].support_tasks[0].properties["source"],
            "a"
        );
    }

    #[test]
    fn parses_zlib_repository_metadata() {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(FIXTURE.as_bytes()).unwrap();
        assert_eq!(
            parse(&encoder.finish().unwrap())
                .unwrap()
                .build_id
                .as_deref(),
            Some("build-1")
        );
    }

    #[test]
    fn rejects_malformed_or_incomplete_metadata() {
        assert!(parse(b"{").is_err());
        assert!(parse(
            br#"{"version":2,"baseProductId":"","installDirectory":"x","products":[],"depots":[]}"#
        )
        .is_err());
        assert!(parse(br#"{"version":1,"baseProductId":"100","installDirectory":"x","products":[{"productId":"100"}],"depots":[{"manifest":"m","productId":"100","languages":[],"size":1}]}"#).is_err());
    }

    #[test]
    fn rejects_oversized_expanded_metadata() {
        let input = vec![b' '; MAX_EXPANDED_METADATA_BYTES + 1];
        assert!(parse(&input).unwrap_err().to_string().contains("64 MiB"));

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&input).unwrap();
        assert!(
            parse(&encoder.finish().unwrap())
                .unwrap_err()
                .to_string()
                .contains("64 MiB")
        );
    }
}
