use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct BuildListResponse {
    #[serde(default)]
    pub items: Vec<BuildResponse>,
}

#[derive(Debug, Deserialize)]
pub struct BuildResponse {
    pub build_id: String,
    pub product_id: String,
    pub os: String,
    pub branch: Option<String>,
    pub version_name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub public: bool,
    pub date_published: Option<String>,
    #[serde(default = "generation_two")]
    pub generation: u32,
    #[serde(default)]
    pub link: String,
}

fn generation_two() -> u32 {
    2
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GenerationTwoRepository {
    #[serde(rename = "version")]
    pub generation: u32,
    #[serde(rename = "baseProductId")]
    pub root_product_id: String,
    #[serde(rename = "buildId")]
    pub build_id: Option<String>,
    pub platform: Option<String>,
    #[serde(rename = "installDirectory")]
    pub install_directory: String,
    pub products: Vec<RepositoryProduct>,
    pub depots: Vec<RepositoryDepot>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryProduct {
    pub product_id: String,
    pub name: Option<String>,
    pub script: Option<String>,
    pub temp_arguments: Option<String>,
    pub temp_executable: Option<String>,
    #[serde(default, alias = "play_tasks")]
    pub play_tasks: Vec<RepositoryTask>,
    #[serde(default, alias = "support_tasks")]
    pub support_tasks: Vec<RepositoryTask>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryDepot {
    #[serde(rename = "manifest")]
    pub manifest_id: String,
    pub product_id: String,
    #[serde(default)]
    pub languages: Vec<String>,
    pub os_bitness: Option<Vec<String>>,
    pub compressed_size: Option<u64>,
    pub size: u64,
    #[serde(default)]
    pub is_gog_depot: bool,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryTask {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: Option<String>,
    pub executable: Option<String>,
    #[serde(default)]
    pub arguments: Vec<String>,
    pub working_directory: Option<String>,
    #[serde(flatten)]
    pub properties: std::collections::BTreeMap<String, Value>,
}
