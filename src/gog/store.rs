use crate::domain::{
    Company, MetadataSource, MetadataTerm, ProductLocalization, ProductMetadata, ProductReference,
    SystemRequirements,
};
use anyhow::{Context, Result};
use std::collections::BTreeMap;

pub fn fetch_wordmark(client: &reqwest::blocking::Client, slug: &str) -> Result<Option<String>> {
    let html = client
        .get(format!("https://www.gog.com/en/game/{slug}"))
        .send()?
        .error_for_status()?
        .text()?;
    Ok(parse_wordmark(&html))
}

fn parse_wordmark(html: &str) -> Option<String> {
    let product_data = html.split_once("window.productcardData")?.1;
    let card = product_data.split_once("cardProductId:")?.0;
    let value = card.rsplit_once("\"logo\":")?.1.trim_start();
    serde_json::Deserializer::from_str(value)
        .into_iter::<serde_json::Value>()
        .next()?
        .ok()?
        .as_str()
        .map(str::to_owned)
}

pub fn fetch(client: &reqwest::blocking::Client, product_id: i64) -> Result<ProductMetadata> {
    let value: serde_json::Value = client
        .get(format!("https://api.gog.com/v2/games/{product_id}"))
        .query(&[("locale", "en-US")])
        .send()?
        .error_for_status()?
        .json()
        .with_context(|| format!("parsing Store API metadata for {product_id}"))?;
    Ok(parse(&value))
}

pub fn parse(value: &serde_json::Value) -> ProductMetadata {
    let embedded = value.get("_embedded").unwrap_or(&serde_json::Value::Null);
    let mut metadata = ProductMetadata {
        tags: terms(embedded, "tags"),
        properties: terms(embedded, "properties"),
        features: terms(embedded, "features"),
        developers: companies(embedded, "developers"),
        publishers: companies(embedded, "publishers"),
        copyright: value
            .get("copyrights")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        store_galaxy_background_url: value
            .pointer("/_links/galaxyBackgroundImage/href")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        store_release_status: value
            .get("releaseStatus")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        store_description: value
            .get("overview")
            .or_else(|| value.get("description"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        ..Default::default()
    };
    if metadata.publishers.is_empty() {
        metadata.publishers = companies(embedded, "publisher");
    }
    metadata.editions = embedded
        .get("editions")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|edition| {
            Some(ProductReference {
                product_id: edition.get("id")?.as_i64()?,
                title: edition.get("name")?.as_str()?.into(),
                relationship: "edition".into(),
            })
        })
        .collect();
    metadata.system_requirements = embedded
        .get("supportedOperatingSystems")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|system| {
            let os = system
                .pointer("/operatingSystem/name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            let requirements = system
                .get("systemRequirements")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let description = |kind: &str| {
                requirements
                    .iter()
                    .find(|entry| {
                        entry.get("type").and_then(serde_json::Value::as_str) == Some(kind)
                    })
                    .and_then(|entry| entry.get("description"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            SystemRequirements {
                operating_system: os,
                minimum: description("minimum"),
                recommended: description("recommended"),
            }
        })
        .collect();
    let mut localizations: BTreeMap<String, ProductLocalization> = BTreeMap::new();
    for item in embedded
        .get("localizations")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let inner = item.get("_embedded").unwrap_or(item);
        let language = inner.get("language").unwrap_or(&serde_json::Value::Null);
        let Some(code) = language.get("code").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let entry = localizations
            .entry(code.into())
            .or_insert(ProductLocalization {
                language_code: code.into(),
                name: language
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(code)
                    .into(),
                text: false,
                audio: false,
            });
        match inner
            .pointer("/localizationScope/type")
            .and_then(serde_json::Value::as_str)
        {
            Some("text") => entry.text = true,
            Some("audio") => entry.audio = true,
            _ => {}
        }
    }
    metadata.localizations = localizations.into_values().collect();
    metadata
}

fn terms(value: &serde_json::Value, field: &str) -> Vec<MetadataTerm> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let item = item
                .get("_embedded")
                .and_then(|v| v.as_object())
                .and_then(|v| v.values().next())
                .unwrap_or(item);
            let name = item
                .get("name")
                .or_else(|| item.get("title"))
                .and_then(serde_json::Value::as_str)?;
            Some(MetadataTerm {
                provider_id: item.get("id").map(value_id),
                name: name.into(),
                slug: item
                    .get("slug")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| slug(name)),
                source: MetadataSource::StoreApi,
            })
        })
        .collect()
}

fn companies(value: &serde_json::Value, field: &str) -> Vec<Company> {
    let Some(value) = value.get(field) else {
        return Vec::new();
    };
    let items: Vec<&serde_json::Value> = value
        .as_array()
        .map(|v| v.iter().collect())
        .unwrap_or_else(|| vec![value]);
    items
        .into_iter()
        .filter_map(|item| {
            let item = item
                .get("_embedded")
                .and_then(|v| v.as_object())
                .and_then(|v| v.values().next())
                .unwrap_or(item);
            Some(Company {
                provider_id: item.get("id").map(value_id),
                name: item.get("name")?.as_str()?.into(),
            })
        })
        .collect()
}

fn value_id(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn slug(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_store_release_status_for_catalog_visibility() {
        let metadata = parse(&serde_json::json!({
            "releaseStatus": "unavailable",
            "overview": "A short DLC description.",
            "_links": {
                "galaxyBackgroundImage": {"href": "https://images.gog.com/clean.jpg"}
            },
            "_embedded": {}
        }));

        assert_eq!(
            metadata.store_release_status.as_deref(),
            Some("unavailable")
        );
        assert_eq!(
            metadata.store_description.as_deref(),
            Some("A short DLC description.")
        );
        assert_eq!(
            metadata.store_galaxy_background_url.as_deref(),
            Some("https://images.gog.com/clean.jpg")
        );
    }

    #[test]
    fn extracts_only_the_store_product_wordmark() {
        let html = r#"
            <script>
            window.productcardData = {
                cardProduct: {"id":"1","logo":"https:\/\/images.gog-statics.com\/wordmark.png"},
                cardProductId: "1"
            };
            </script>
        "#;
        assert_eq!(
            parse_wordmark(html).as_deref(),
            Some("https://images.gog-statics.com/wordmark.png")
        );
    }

    #[test]
    fn missing_store_wordmark_uses_the_text_fallback() {
        let html = r#"
            window.productcardData = {
                cardProduct: {"id":"1","logo":null},
                cardProductId: "1"
            };
        "#;
        assert_eq!(parse_wordmark(html), None);
    }
}
