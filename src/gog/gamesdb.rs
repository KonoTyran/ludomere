use crate::domain::{Company, MetadataSource, MetadataTerm, ProductMetadata};
use anyhow::{Context, Result};

pub fn fetch(
    client: &reqwest::blocking::Client,
    product_id: i64,
) -> Result<Option<ProductMetadata>> {
    let response = client
        .get(format!(
            "https://gamesdb.gog.com/platforms/gog/external_releases/{product_id}"
        ))
        .send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        // GamesDB is supplemental enrichment and does not map every GOG
        // product, particularly some DLC and non-game products.
        return Ok(None);
    }
    let value: serde_json::Value = response
        .error_for_status()?
        .json()
        .with_context(|| format!("parsing GamesDB metadata for {product_id}"))?;
    Ok(Some(parse(&value)))
}

pub fn parse(value: &serde_json::Value) -> ProductMetadata {
    let game = value.get("game").unwrap_or(value);
    ProductMetadata {
        genres: terms(game, "genres"),
        themes: terms(game, "themes"),
        game_modes: terms(game, "game_modes"),
        developers: companies(game, "developers"),
        publishers: companies(game, "publishers"),
        gamesdb_summary: game.get("summary").and_then(localized_string),
        gamesdb_artwork_url: game
            .get("artworks")
            .and_then(serde_json::Value::as_array)
            .and_then(|artworks| artworks.first())
            .and_then(|artwork| image_url_from_value(artwork, "jpg")),
        gamesdb_horizontal_artwork_url: image_url(game, "horizontal_artwork", "jpg"),
        gamesdb_background_url: image_url(game, "background", "jpg"),
        gamesdb_media_checked: true,
        gamesdb_media_version: 2,
        ..Default::default()
    }
}

fn image_url(value: &serde_json::Value, field: &str, extension: &str) -> Option<String> {
    image_url_from_value(value.get(field)?, extension)
}

fn image_url_from_value(value: &serde_json::Value, extension: &str) -> Option<String> {
    value
        .get("url_format")?
        .as_str()
        .map(|url| url.replace("{formatter}", "").replace("{ext}", extension))
}

fn terms(value: &serde_json::Value, field: &str) -> Vec<MetadataTerm> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = localized_string(item.get("name")?)?;
            Some(MetadataTerm {
                provider_id: item.get("id").map(id),
                name: name.clone(),
                slug: item
                    .get("slug")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&name)
                    .to_lowercase()
                    .replace(' ', "-"),
                source: MetadataSource::GamesDb,
            })
        })
        .collect()
}
fn localized_string(value: &serde_json::Value) -> Option<String> {
    value.as_str().map(str::to_owned).or_else(|| {
        value
            .get("en-US")
            .or_else(|| value.get("*"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    })
}

fn companies(value: &serde_json::Value, field: &str) -> Vec<Company> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(Company {
                provider_id: item.get("id").map(id),
                name: item.get("name")?.as_str()?.into(),
            })
        })
        .collect()
}
fn id(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_gamesdb_hero_assets() {
        let metadata = parse(&serde_json::json!({
            "game": {
                "artworks": [{
                    "url_format": "https://images.gog.com/artwork{formatter}.{ext}?namespace=gamesdb"
                }],
                "horizontal_artwork": {
                    "url_format": "https://images.gog.com/horizontal{formatter}.{ext}?namespace=gamesdb"
                },
                "background": {
                    "url_format": "https://images.gog.com/background{formatter}.{ext}?namespace=gamesdb"
                },
            }
        }));

        assert_eq!(
            metadata.gamesdb_artwork_url.as_deref(),
            Some("https://images.gog.com/artwork.jpg?namespace=gamesdb")
        );
        assert_eq!(
            metadata.gamesdb_horizontal_artwork_url.as_deref(),
            Some("https://images.gog.com/horizontal.jpg?namespace=gamesdb")
        );
        assert_eq!(
            metadata.gamesdb_background_url.as_deref(),
            Some("https://images.gog.com/background.jpg?namespace=gamesdb")
        );
        assert_eq!(metadata.gamesdb_media_version, 2);
    }
}
