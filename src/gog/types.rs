use serde::Deserialize;

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
