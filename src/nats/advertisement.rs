use anyhow::Result;
use async_nats::Client;

pub const SITE_ADVERTISEMENT_SUBJECT: &str = "fed.site.advertisement";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SiteAdvertisement {
    pub site: String,
    pub repo_url: String,
    pub commit_hash: String,
}

pub async fn publish_site_advertisement(client: &Client, msg: &SiteAdvertisement) -> Result<()> {
    let payload = serde_json::to_vec(msg)?;
    client
        .publish(SITE_ADVERTISEMENT_SUBJECT.to_string(), payload.into())
        .await?;
    Ok(())
}
