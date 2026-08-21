use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountTag {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountLibraryOrganization {
    pub tags: Vec<AccountTag>,
    pub assignments: Vec<(i64, String)>,
    pub hidden_product_ids: Vec<i64>,
}

pub fn library_organization(
    client: &reqwest::blocking::Client,
    token: &crate::auth::Token,
) -> Result<AccountLibraryOrganization> {
    let visible = pages(client, token, false)?;
    let hidden = pages(client, token, true)?;
    let mut result = AccountLibraryOrganization::default();
    for page in visible.iter().chain(&hidden) {
        if result.tags.is_empty() {
            result.tags = parse_tags(page.get("tags"));
        }
        for product in page
            .get("products")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(product_id) = product.get("id").and_then(parse_i64) else {
                continue;
            };
            for tag_id in product
                .get("tags")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(parse_id)
            {
                result.assignments.push((product_id, tag_id));
            }
        }
    }
    for page in hidden {
        result.hidden_product_ids.extend(
            page.get("products")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|product| product.get("id"))
                .filter_map(parse_i64),
        );
    }
    result.tags.sort_by(|left, right| left.id.cmp(&right.id));
    result.tags.dedup_by(|left, right| left.id == right.id);
    result.assignments.sort();
    result.assignments.dedup();
    result.hidden_product_ids.sort_unstable();
    result.hidden_product_ids.dedup();
    Ok(result)
}

fn pages(
    client: &reqwest::blocking::Client,
    token: &crate::auth::Token,
    hidden: bool,
) -> Result<Vec<serde_json::Value>> {
    let mut pages = Vec::new();
    for page in 1..=1_000_u32 {
        let value: serde_json::Value = client
            .get("https://embed.gog.com/account/getFilteredProducts")
            .query(&[
                ("hiddenFlag", u8::from(hidden).to_string()),
                ("isUpdated", "0".into()),
                ("mediaType", "1".into()),
                ("sortBy", "title".into()),
                ("system", String::new()),
                ("page", page.to_string()),
            ])
            .bearer_auth(&token.access_token)
            .send()?
            .error_for_status()?
            .json()
            .context("decoding GOG account library organization")?;
        let current = value.get("page").and_then(parse_u32).unwrap_or(page);
        let total = value
            .get("totalPages")
            .and_then(parse_u32)
            .unwrap_or(current);
        pages.push(value);
        if current >= total || total == 0 {
            return Ok(pages);
        }
    }
    bail!("GOG account library pagination exceeded its safety limit")
}

fn parse_tags(value: Option<&serde_json::Value>) -> Vec<AccountTag> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tag| {
            Some(AccountTag {
                id: parse_id(tag.get("id")?)?,
                name: tag.get("name")?.as_str()?.trim().to_owned(),
            })
        })
        .filter(|tag| !tag.name.is_empty())
        .collect()
}

fn parse_id(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|value| value.to_string()))
}

fn parse_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn parse_u32(value: &serde_json::Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mixed_account_tag_identifiers() {
        let value: serde_json::Value =
            serde_json::from_str(r#"{"tags":[{"id":7,"name":"RPG"},{"id":"8","name":"Co-op"}]}"#)
                .unwrap();
        assert_eq!(
            parse_tags(value.get("tags")),
            [
                AccountTag {
                    id: "7".into(),
                    name: "RPG".into()
                },
                AccountTag {
                    id: "8".into(),
                    name: "Co-op".into()
                },
            ]
        );
    }
}
