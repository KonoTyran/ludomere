use crate::{domain::GalaxyBuild, gog::types::BuildListResponse};
use anyhow::{Context, Result};
use chrono::DateTime;

pub fn fetch(
    client: &reqwest::blocking::Client,
    product_id: i64,
    operating_system: &str,
) -> Result<Vec<GalaxyBuild>> {
    fetch_from(
        client,
        None,
        None,
        2,
        "https://content-system.gog.com",
        product_id,
        operating_system,
    )
}

pub fn fetch_authenticated(
    client: &reqwest::blocking::Client,
    access_token: &str,
    branch: crate::gog::depot_acquisition::BranchAccess<'_>,
    product_id: i64,
    operating_system: &str,
) -> Result<Vec<GalaxyBuild>> {
    fetch_authenticated_generation(
        client,
        access_token,
        branch.password,
        product_id,
        operating_system,
        2,
    )
}

pub fn fetch_authenticated_generation(
    client: &reqwest::blocking::Client,
    access_token: &str,
    password: Option<&str>,
    product_id: i64,
    operating_system: &str,
    generation: u32,
) -> Result<Vec<GalaxyBuild>> {
    if !matches!(generation, 1 | 2) {
        anyhow::bail!("unsupported Galaxy build-list generation");
    }
    fetch_from(
        client,
        Some(access_token),
        password,
        generation,
        "https://content-system.gog.com",
        product_id,
        operating_system,
    )
}

fn fetch_from(
    client: &reqwest::blocking::Client,
    access_token: Option<&str>,
    password: Option<&str>,
    generation: u32,
    base_url: &str,
    product_id: i64,
    operating_system: &str,
) -> Result<Vec<GalaxyBuild>> {
    let mut request = client.get(format!(
        "{base_url}/products/{product_id}/os/{operating_system}/builds"
    ));
    if let Some(token) = access_token {
        request = request.bearer_auth(token);
    }
    let generation = generation.to_string();
    request = request.query(&[("generation", generation.as_str())]);
    if let Some(password) = password {
        request = request.query(&[("password", password)]);
    }
    let response = request.send()?;
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(
            crate::gog::depot_acquisition::AcquisitionError(if password.is_some() {
                crate::gog::depot_acquisition::AcquisitionErrorKind::InvalidBranchPassword
            } else {
                crate::gog::depot_acquisition::AcquisitionErrorKind::NeedsBranchPassword
            })
            .into(),
        );
    }
    let response: BuildListResponse = response
        .error_for_status()?
        .json()
        .with_context(|| format!("parsing {operating_system} builds for {product_id}"))?;
    let now = chrono::Utc::now().timestamp();
    Ok(response
        .items
        .into_iter()
        .map(|build| GalaxyBuild {
            build_id: build.build_id,
            product_id: build.product_id.parse().unwrap_or(product_id),
            operating_system: build.os,
            version: build.version_name,
            branch: build.branch,
            tags: build.tags,
            public: build.public,
            generation: build.generation,
            repository_id: build.link.rsplit('/').next().map(str::to_owned),
            repository_url: build.link,
            published_at: build.date_published.as_deref().and_then(parse_date),
            currently_returned: true,
            first_seen_at: now,
            last_seen_at: now,
        })
        .collect())
}

fn parse_date(value: &str) -> Option<i64> {
    DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%z")
        .ok()
        .map(|date| date.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn server(status: &str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            let size = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]).into_owned();
            let body = r#"{"items":[]}"#;
            write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            request
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn authenticated_fetch_encodes_password_and_sends_bearer() {
        let (base, handle) = server("200 OK");
        fetch_from(
            &reqwest::blocking::Client::new(),
            Some("token"),
            Some("p@ss word&?"),
            2,
            &base,
            7,
            "windows",
        )
        .unwrap();
        let request = handle.join().unwrap();
        assert!(
            request.contains("authorization: Bearer token")
                || request.contains("Authorization: Bearer token")
        );
        assert!(
            request.contains("password=p%40ss+word%26%3F")
                || request.contains("password=p%40ss%20word%26%3F")
        );
        assert!(!request.contains("p@ss word&?"));
    }

    #[test]
    fn forbidden_build_list_is_typed_without_secrets() {
        for (password, expected) in [
            (
                None,
                crate::gog::depot_acquisition::AcquisitionErrorKind::NeedsBranchPassword,
            ),
            (
                Some("secret!?"),
                crate::gog::depot_acquisition::AcquisitionErrorKind::InvalidBranchPassword,
            ),
        ] {
            let (base, handle) = server("403 Forbidden");
            let error = fetch_from(
                &reqwest::blocking::Client::new(),
                Some("token"),
                password,
                2,
                &base,
                7,
                "windows",
            )
            .unwrap_err();
            handle.join().unwrap();
            assert_eq!(
                error
                    .downcast_ref::<crate::gog::depot_acquisition::AcquisitionError>()
                    .unwrap()
                    .0,
                expected
            );
            assert!(!format!("{error:?}").contains("secret!?"));
        }
    }

    #[test]
    fn authenticated_generation_one_listing_is_visible() {
        let (base, handle) = server("200 OK");
        fetch_from(
            &reqwest::blocking::Client::new(),
            Some("token"),
            None,
            1,
            &base,
            7,
            "windows",
        )
        .unwrap();
        assert!(handle.join().unwrap().contains("generation=1"));
    }
}
