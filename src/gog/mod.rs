pub mod account;
pub mod audit;
pub mod builds;
pub mod capabilities;
pub mod capability_audit;
pub mod depot_acquisition;
pub mod depot_manifest;
pub mod depot_service;
pub mod friends;
pub mod gameplay;
pub mod gamesdb;
pub mod presence;
pub mod product;
pub mod repository;
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
