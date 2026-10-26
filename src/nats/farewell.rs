use anyhow::Result;
use async_nats::Client;

pub const SITE_FAREWELL_SUBJECT: &str = "fed.site.farewell";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SiteFarewell {
    pub site: String,
    pub repo_url: String,
}

pub async fn publish_site_farewell(client: &Client, msg: &SiteFarewell) -> Result<()> {
    let payload = serde_json::to_vec(msg)?;
    client
        .publish(SITE_FAREWELL_SUBJECT.to_string(), payload.into())
        .await?;

    client.flush().await?;
    Ok(())
}
