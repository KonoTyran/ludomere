use crate::{domain::GalaxyBuild, gog::types::BuildListResponse};
use anyhow::{Context, Result};
use chrono::DateTime;

pub fn fetch(
    client: &reqwest::blocking::Client,
    product_id: i64,
    operating_system: &str,
) -> Result<Vec<GalaxyBuild>> {
    let response: BuildListResponse = client
        .get(format!(
            "https://content-system.gog.com/products/{product_id}/os/{operating_system}/builds"
        ))
        .query(&[("generation", "2")])
        .send()?
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
