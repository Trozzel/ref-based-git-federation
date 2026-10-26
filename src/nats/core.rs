use anyhow::Result;
use async_nats::Client;

pub async fn connect(nats_url: &str) -> Result<Client> {
    let client = async_nats::connect(nats_url).await?;
    Ok(client)
}
