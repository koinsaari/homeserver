use crate::config::AlertsConfig;
use crate::watcher::FileEvent;
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

pub async fn send_alert_for_event(
    client: &reqwest::Client,
    config: &AlertsConfig,
    event: &FileEvent,
) {
    if !config.enabled {
        return;
    }

    let message = match event {
        FileEvent::Organized { old_path, new_path } => {
            let filename = old_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| old_path.display().to_string());
            let dest = new_path.display();
            format!("Organized: {filename} → {dest}")
        }
        _ => return,
    };

    if let Err(e) = send_alert(client, config, &message).await {
        warn!(error = %e, "failed to send ntfy alert");
    }
}
