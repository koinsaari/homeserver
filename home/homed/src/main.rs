mod alerts;
mod checks;
mod config;
mod metadata;
mod nextcloud;
mod organizer;
mod scanner;
mod watcher;

use std::time::Duration;

use config::Config;
use tokio::sync::{broadcast, mpsc};
use tokio::time::Instant;
use tracing::{error, info, warn};
use watcher::FileEvent;

use alerts::send_batch_alert;

const BATCH_QUIET_PERIOD: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    info!("homed starting up");

    let config = Config::load("/opt/homed/config.toml")?;

    let http_client = reqwest::Client::new();
    let alerts_config = config.alerts.clone();

    let (shutdown_tx, _) = broadcast::channel(1);
    let (output_tx, mut output_rx) = mpsc::channel::<FileEvent>(100);

    let photos_handles = spawn_photos_pipeline(&config, &shutdown_tx, output_tx.clone());
    let media_handles = spawn_media_pipeline(&config, &shutdown_tx, output_tx);

    info!("pipelines running");

    let mut organized_count = 0usize;
    let mut unsorted_count = 0usize;
    let mut failed_count = 0usize;
    let mut last_event_time: Option<Instant> = None;

    loop {
        let timeout = last_event_time
            .map(|t| BATCH_QUIET_PERIOD.saturating_sub(t.elapsed()))
            .unwrap_or(BATCH_QUIET_PERIOD);

        tokio::select! {
            Some(event) = output_rx.recv() => {
                log_event(&event);
                match &event {
                    FileEvent::Organized { .. } => organized_count += 1,
                    FileEvent::Unsorted { .. } => unsorted_count += 1,
                    FileEvent::Failed { .. } => failed_count += 1,
                    _ => {}
                }
                last_event_time = Some(Instant::now());
            }
            _ = tokio::time::sleep(timeout), if last_event_time.is_some() => {
                if last_event_time.map(|t| t.elapsed() >= BATCH_QUIET_PERIOD).unwrap_or(false) {
                    send_batch_alert(
                        &http_client,
                        &alerts_config,
                        organized_count,
                        unsorted_count,
                        failed_count,
                    ).await;
                    organized_count = 0;
                    unsorted_count = 0;
                    failed_count = 0;
                    last_event_time = None;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("received shutdown signal, draining pipelines");
                if organized_count > 0 || unsorted_count > 0 || failed_count > 0 {
                    send_batch_alert(
                        &http_client,
                        &alerts_config,
                        organized_count,
                        unsorted_count,
                        failed_count,
                    ).await;
                }
                shutdown_tx.send(()).ok();
                break;
            }
        }
    }

    let shutdown_timeout = std::time::Duration::from_secs(30);
    let all_handles = async {
        for handle in photos_handles.into_iter().chain(media_handles) {
            let _ = handle.await;
        }
    };

    if tokio::time::timeout(shutdown_timeout, all_handles)
        .await
        .is_err()
    {
        warn!("shutdown timed out after 30s, forcing exit");
    } else {
        info!("shutdown complete");
    }

    Ok(())
}

fn spawn_photos_pipeline(
    config: &Config,
    shutdown_tx: &broadcast::Sender<()>,
    output_tx: mpsc::Sender<FileEvent>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let (watcher_tx, watcher_rx) = mpsc::channel(100);
    let (metadata_tx, metadata_rx) = mpsc::channel(100);
    let (organizer_tx, organizer_rx) = mpsc::channel(100);

    let watcher_handle = tokio::spawn({
        let config = config.photos.watcher.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) = watcher::run_watcher(config, watcher_tx, shutdown_rx).await {
                error!(error = %e, "photos watcher failed");
            }
        }
    });

    let metadata_handle = tokio::spawn({
        let config = config.photos.organizer.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) =
                metadata::run_metadata(config, watcher_rx, metadata_tx, shutdown_rx).await
            {
                error!(error = %e, "photos metadata failed");
            }
        }
    });

    let organizer_handle = tokio::spawn({
        let config = config.photos.organizer.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) =
                organizer::run_organizer(config, metadata_rx, organizer_tx, shutdown_rx).await
            {
                error!(error = %e, "photos organizer failed");
            }
        }
    });

    let nextcloud_handle = tokio::spawn({
        let config = config.photos.nextcloud.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) =
                nextcloud::run_nextcloud(config, organizer_rx, output_tx, shutdown_rx).await
            {
                error!(error = %e, "photos nextcloud failed");
            }
        }
    });

    vec![
        watcher_handle,
        metadata_handle,
        organizer_handle,
        nextcloud_handle,
    ]
}

fn spawn_media_pipeline(
    config: &Config,
    shutdown_tx: &broadcast::Sender<()>,
    output_tx: mpsc::Sender<FileEvent>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let (watcher_tx, watcher_rx) = mpsc::channel(100);

    let watcher_handle = tokio::spawn({
        let config = config.media.watcher.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) = watcher::run_watcher(config, watcher_tx, shutdown_rx).await {
                error!(error = %e, "media watcher failed");
            }
        }
    });

    let scanner_handle = tokio::spawn({
        let config = config.media.scanner.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) = scanner::run_scanner(config, watcher_rx, output_tx, shutdown_rx).await {
                error!(error = %e, "media scanner failed");
            }
        }
    });

    vec![watcher_handle, scanner_handle]
}

fn log_event(event: &FileEvent) {
    match event {
        FileEvent::Detected { path, size } => {
            info!(path = %path.display(), size, "file detected");
        }
        FileEvent::Scanned { path, clean } => {
            if *clean {
                info!(path = %path.display(), "scan passed");
            } else {
                warn!(path = %path.display(), "malware detected");
            }
        }
        FileEvent::Enriched {
            path,
            media_type,
            datetime,
        } => {
            info!(
                path = %path.display(),
                media_type = ?media_type,
                datetime = %datetime,
                "metadata extracted"
            );
        }
        FileEvent::Organized { old_path, new_path } => {
            info!(
                from = %old_path.display(),
                to = %new_path.display(),
                "file organized"
            );
        }
        FileEvent::Cleaned { path, reason } => {
            info!(path = %path.display(), reason, "file cleaned");
        }
        FileEvent::Unsorted { path, media_type } => {
            info!(
                path = %path.display(),
                media_type = ?media_type,
                "no valid date, moving to unsorted"
            );
        }
        FileEvent::Failed { path, error } => {
            warn!(path = %path.display(), error, "processing failed");
        }
    }
}
