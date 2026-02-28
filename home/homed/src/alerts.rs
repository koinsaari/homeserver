use crate::config::AlertsConfig;
use tracing::warn;

pub async fn send_alert(
    client: &reqwest::Client,
    config: &AlertsConfig,
    message: &str,
) -> Result<(), reqwest::Error> {
    client
        .post(format!("{}/{}", config.url, config.topic))
        .bearer_auth(&config.token)
        .body(message.to_string())
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

pub async fn send_batch_alert(
    client: &reqwest::Client,
    config: &AlertsConfig,
    organized: usize,
    unsorted: usize,
    failed: usize,
) {
    if !config.enabled {
        return;
    }

    if organized == 0 && unsorted == 0 && failed == 0 {
        return;
    }

    let mut parts = Vec::new();
    if organized > 0 {
        parts.push(format!("{} organized", organized));
    }
    if unsorted > 0 {
        parts.push(format!("{} unsorted", unsorted));
    }
    if failed > 0 {
        parts.push(format!("{} failed", failed));
    }

    let message = format!("Photos: {}", parts.join(", "));

    if let Err(e) = send_alert(client, config, &message).await {
        warn!(error = %e, "failed to send ntfy alert");
    }
}
