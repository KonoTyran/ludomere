use crate::{
    domain::GalaxyBuild,
    gog::{
        depot_manifest::DepotManifest,
        types::{GenerationTwoRepository, RepositoryDepot},
    },
};
use anyhow::{Context, Result, bail};
use std::{collections::BTreeSet, io::Read};

const MAX_COMPRESSED_METADATA_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquisitionErrorKind {
    GenerationOneUnsupported,
    NeedsBranchPassword,
    InvalidBranchPassword,
}

#[derive(Debug)]
pub struct AcquisitionError(pub AcquisitionErrorKind);

impl std::fmt::Display for AcquisitionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self.0 {
            AcquisitionErrorKind::GenerationOneUnsupported => {
                "generation-1 Galaxy builds are unsupported"
            }
            AcquisitionErrorKind::NeedsBranchPassword => "the protected branch requires a password",
            AcquisitionErrorKind::InvalidBranchPassword => {
                "the protected branch password was rejected"
            }
        })
    }
}

impl std::error::Error for AcquisitionError {}

#[derive(Clone)]
pub struct BranchAccess<'a> {
    pub password: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Selection {
    pub language: String,
    pub bitness: Option<String>,
    pub owned_dlc: BTreeSet<i64>,
    pub selected_dlc: BTreeSet<i64>,
}

#[derive(Debug, Clone)]
pub struct SelectedSource {
    pub product_id: i64,
    pub depot_identity: String,
    pub manifest_id: String,
    pub content_root: String,
    pub manifest_bytes: Vec<u8>,
    pub manifest: DepotManifest,
}

#[derive(Debug)]
pub struct Acquisition {
    pub build_id: String,
    pub branch: Option<String>,
    pub repository_id: String,
    pub repository: GenerationTwoRepository,
    pub sources: Vec<SelectedSource>,
    pub entitlement_only_dlc: Vec<i64>,
}

pub fn acquire(
    client: &reqwest::blocking::Client,
    access_token: &str,
    build: &GalaxyBuild,
    selection: &Selection,
) -> Result<Acquisition> {
    acquire_with_meta_base(client, access_token, build, selection, META_BASE_URL)
}

fn acquire_with_meta_base(
    client: &reqwest::blocking::Client,
    access_token: &str,
    build: &GalaxyBuild,
    selection: &Selection,
    meta_base: &str,
) -> Result<Acquisition> {
    validate_build_access(build)?;
    let repository_bytes = fetch_bytes(client, access_token, &build.repository_url)?;
    let repository = crate::gog::repository::parse(&repository_bytes)?;
    if repository
        .build_id
        .as_deref()
        .is_some_and(|id| id != build.build_id)
    {
        bail!("repository build identity does not match the selected build");
    }
    let selected = select_depots(&repository, selection)?;
    let mut sources = Vec::with_capacity(selected.len());
    for depot in selected {
        let url = manifest_meta_url_at(meta_base, &depot.manifest_id)?;
        let bytes = fetch_bytes(client, access_token, &url)?;
        let manifest = crate::gog::depot_manifest::parse(&bytes)?;
        sources.push(SelectedSource {
            product_id: depot
                .product_id
                .parse()
                .context("repository depot product ID is invalid")?,
            depot_identity: depot.manifest_id.clone(),
            manifest_id: depot.manifest_id.clone(),
            content_root: "/".into(),
            manifest_bytes: bytes,
            manifest,
        });
    }
    let payload_products = sources
        .iter()
        .map(|source| source.product_id)
        .collect::<BTreeSet<_>>();
    let entitlement_only_dlc = entitlement_only(selection, &payload_products);
    Ok(Acquisition {
        build_id: build.build_id.clone(),
        branch: build.branch.clone(),
        repository_id: build
            .repository_id
            .clone()
            .context("selected build has no repository identity")?,
        repository,
        sources,
        entitlement_only_dlc,
    })
}

fn entitlement_only(selection: &Selection, payload_products: &BTreeSet<i64>) -> Vec<i64> {
    selection
        .selected_dlc
        .iter()
        .filter(|id| selection.owned_dlc.contains(id) && !payload_products.contains(id))
        .copied()
        .collect()
}

pub fn select_depots<'a>(
    repository: &'a GenerationTwoRepository,
    selection: &Selection,
) -> Result<Vec<&'a RepositoryDepot>> {
    let root: i64 = repository
        .root_product_id
        .parse()
        .context("root product ID is invalid")?;
    if !selection.selected_dlc.is_subset(&selection.owned_dlc) {
        bail!("selected DLC is not owned");
    }
    let matches = |depot: &&RepositoryDepot| {
        (depot.languages.is_empty()
            || depot
                .languages
                .iter()
                .any(|value| language_matches(value, &selection.language)))
            && (selection.bitness.is_none()
                || depot.os_bitness.as_ref().is_none_or(|values| {
                    values
                        .iter()
                        .any(|value| Some(value) == selection.bitness.as_ref())
                }))
    };
    let mut selected = repository
        .depots
        .iter()
        .filter(|depot| depot.product_id.parse::<i64>() == Ok(root))
        .filter(matches)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        bail!("repository has no matching base depot");
    }
    selected.extend(
        repository
            .depots
            .iter()
            .filter(|depot| {
                depot
                    .product_id
                    .parse::<i64>()
                    .is_ok_and(|id| selection.selected_dlc.contains(&id))
            })
            .filter(matches),
    );
    Ok(selected)
}

fn language_matches(depot: &str, selected: &str) -> bool {
    if depot == "*" || depot.eq_ignore_ascii_case(selected) {
        return true;
    }
    let depot_regional = depot.contains('-');
    let selected_regional = selected.contains('-');
    (!depot_regional || !selected_regional)
        && depot
            .split('-')
            .next()
            .zip(selected.split('-').next())
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn validate_build_access(build: &GalaxyBuild) -> Result<()> {
    if build.generation == 1 {
        return Err(AcquisitionError(AcquisitionErrorKind::GenerationOneUnsupported).into());
    }
    if build.generation != 2 {
        bail!("unsupported Galaxy build generation {}", build.generation);
    }
    if build.repository_url.is_empty() || build.repository_id.as_deref().is_none_or(str::is_empty) {
        bail!("selected build repository identity is missing");
    }
    Ok(())
}

pub const META_BASE_URL: &str = "https://gog-cdn-fastly.gog.com/content-system/v2/meta/";

pub fn manifest_meta_url(reference: &str) -> Result<String> {
    manifest_meta_url_at(META_BASE_URL, reference)
}

fn manifest_meta_url_at(base: &str, reference: &str) -> Result<String> {
    if reference.is_empty()
        || reference.starts_with('/')
        || reference.contains("..")
        || reference.contains(['?', '#', '\\'])
        || !reference
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'.'))
    {
        bail!("invalid depot manifest reference");
    }
    let path = if reference.contains('/') {
        reference.to_owned()
    } else {
        if reference.len() < 4 || !reference.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("unsharded depot manifest reference is invalid");
        }
        format!("{}/{}/{}", &reference[..2], &reference[2..4], reference)
    };
    Ok(format!(
        "{}{path}",
        base.trim_end_matches('/').to_owned() + "/"
    ))
}

fn fetch_bytes(client: &reqwest::blocking::Client, token: &str, url: &str) -> Result<Vec<u8>> {
    let mut response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|_| anyhow::anyhow!("authenticated Galaxy metadata request failed"))?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_COMPRESSED_METADATA_BYTES)
    {
        bail!("compressed Galaxy metadata exceeds the safety limit");
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(MAX_COMPRESSED_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("reading authenticated Galaxy metadata failed"))?;
    if bytes.len() as u64 > MAX_COMPRESSED_METADATA_BYTES {
        bail!("compressed Galaxy metadata exceeds the safety limit");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn repository() -> GenerationTwoRepository {
        crate::gog::repository::parse(br#"{
          "version":2,"baseProductId":"100","buildId":"build","platform":"windows","installDirectory":"Game",
          "products":[{"productId":"100"},{"productId":"200"},{"productId":"300"}],
          "depots":[
            {"manifest":"base-neutral","productId":"100","languages":[],"size":1},
            {"manifest":"base-en64","productId":"100","languages":["en-US"],"osBitness":["64"],"size":1},
            {"manifest":"dlc-200","productId":"200","languages":["en-US"],"osBitness":["64"],"size":1}
          ]}"#).unwrap()
    }

    fn selection() -> Selection {
        Selection {
            language: "en-us".into(),
            bitness: Some("64".into()),
            owned_dlc: BTreeSet::from([200, 300]),
            selected_dlc: BTreeSet::from([200, 300]),
        }
    }

    fn build(generation: u32, protected: bool) -> GalaxyBuild {
        GalaxyBuild {
            build_id: "build".into(),
            product_id: 100,
            operating_system: "windows".into(),
            version: None,
            branch: protected.then(|| "beta".into()),
            tags: Vec::new(),
            public: !protected,
            generation,
            repository_url: "https://example/repository".into(),
            repository_id: Some("repository".into()),
            published_at: None,
            currently_returned: true,
            first_seen_at: 0,
            last_seen_at: 0,
        }
    }

    #[test]
    fn selects_base_then_owned_dlc_in_repository_order() {
        let repository = repository();
        let selected = select_depots(&repository, &selection()).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|depot| depot.manifest_id.as_str())
                .collect::<Vec<_>>(),
            ["base-neutral", "base-en64", "dlc-200"]
        );
        assert_eq!(
            entitlement_only(&selection(), &BTreeSet::from([100, 200])),
            [300]
        );
    }

    #[test]
    fn generic_catalog_language_matches_regional_and_wildcard_depots() {
        assert!(language_matches("en-US", "en"));
        assert!(language_matches("*", "en"));
        assert!(!language_matches("pt-BR", "pt-PT"));
        let mut selected = selection();
        selected.language = "en".into();
        assert_eq!(select_depots(&repository(), &selected).unwrap().len(), 3);
    }

    #[test]
    fn filters_language_bitness_and_rejects_unowned_dlc() {
        let mut selected = selection();
        selected.bitness = Some("32".into());
        assert_eq!(select_depots(&repository(), &selected).unwrap().len(), 1);
        selected.selected_dlc.insert(400);
        assert!(select_depots(&repository(), &selected).is_err());
    }

    #[test]
    fn generation_and_protected_branch_outcomes_are_typed() {
        let error = validate_build_access(&build(1, false)).unwrap_err();
        assert_eq!(
            error.downcast_ref::<AcquisitionError>().unwrap().0,
            AcquisitionErrorKind::GenerationOneUnsupported
        );
        let protected = build(2, true);
        validate_build_access(&protected).unwrap();
    }

    #[test]
    fn rejects_missing_repository_identity() {
        let mut value = build(2, false);
        value.repository_id = None;
        assert!(validate_build_access(&value).is_err());
    }

    #[test]
    fn derives_validated_manifest_meta_urls() {
        assert_eq!(
            manifest_meta_url("0123456789abcdef").unwrap(),
            format!("{META_BASE_URL}01/23/0123456789abcdef")
        );
        assert_eq!(
            manifest_meta_url("custom/path/ref").unwrap(),
            format!("{META_BASE_URL}custom/path/ref")
        );
        for invalid in ["", "/root", "../escape", "abcd?token=secret", "xyz"] {
            assert!(manifest_meta_url(invalid).is_err());
        }
    }

    #[test]
    fn authenticated_metadata_read_is_bounded_and_secret_safe() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let _ = stream.read(&mut request).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\nx",
                MAX_COMPRESSED_METADATA_BYTES + 1
            )
            .unwrap();
        });
        let url = format!("http://{address}/secret-url");
        let error =
            fetch_bytes(&reqwest::blocking::Client::new(), "secret-token", &url).unwrap_err();
        server.join().unwrap();
        let message = format!("{error:?}");
        assert!(message.contains("safety limit"));
        assert!(!message.contains("secret-token"));
        assert!(!message.contains("secret-url"));
    }

    #[test]
    fn acquisition_derives_ordered_manifest_paths_and_root() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for index in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let size = stream.read(&mut request).unwrap();
                requests.push(String::from_utf8_lossy(&request[..size]).into_owned());
                let body = if index == 0 {
                    r#"{"version":2,"baseProductId":"100","buildId":"build","platform":"windows","installDirectory":"Game","products":[{"productId":"100"},{"productId":"200"}],"depots":[{"manifest":"0123456789abcdef","productId":"100","languages":[],"size":1},{"manifest":"custom/ref","productId":"200","languages":[],"size":1}]}"#
                } else {
                    r#"{"version":2,"depot":{"items":[]}}"#
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            requests
        });
        let base = format!("http://{address}");
        let mut build = build(2, false);
        build.repository_url = format!("{base}/repository");
        let selection = Selection {
            language: "en".into(),
            bitness: None,
            owned_dlc: BTreeSet::from([200]),
            selected_dlc: BTreeSet::from([200]),
        };
        let acquisition = acquire_with_meta_base(
            &reqwest::blocking::Client::new(),
            "token",
            &build,
            &selection,
            &format!("{base}/meta/"),
        )
        .unwrap();
        assert_eq!(
            acquisition
                .sources
                .iter()
                .map(|source| source.manifest_id.as_str())
                .collect::<Vec<_>>(),
            ["0123456789abcdef", "custom/ref"]
        );
        assert!(
            acquisition
                .sources
                .iter()
                .all(|source| source.content_root == "/")
        );
        let requests = server.join().unwrap();
        assert!(requests[1].starts_with("GET /meta/01/23/0123456789abcdef "));
        assert!(requests[2].starts_with("GET /meta/custom/ref "));
        assert!(
            requests
                .iter()
                .all(|request| request.contains("Bearer token"))
        );
    }
}
