pub mod audit;
pub mod builds;
pub mod gamesdb;
pub mod product;
pub mod store;
pub mod types;

use anyhow::Result;
use std::time::Duration;

pub fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(45))
        .user_agent(crate::identity::USER_AGENT)
        .build()?)
}
