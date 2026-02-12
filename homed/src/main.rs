mod config;
mod metadata;
mod nextcloud;
mod organizer;
mod scanner;
mod watcher;

use config::Config;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn, error};
use watcher::FileEvent;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    info!("homed starting up");

    let config = Config::load("config.toml")?;
    info!(
        paths = config.watcher.paths.len(),
        debounce_ms = config.watcher.debounce_ms,
        scanner = config.scanner.enabled,
        organizer = config.organizer.enabled,
        nextcloud = config.nextcloud.enabled,
        "configuration loaded"
    );

    let (shutdown_tx, _) = broadcast::channel(1);

    let (watcher_tx, watcher_rx) = mpsc::channel(100);
    let (scanner_tx, scanner_rx) = mpsc::channel(100);
    let (metadata_tx, metadata_rx) = mpsc::channel(100);
    let (organizer_tx, organizer_rx) = mpsc::channel(100);
    let (nextcloud_tx, mut output_rx) = mpsc::channel(100);

    let watcher_handle = tokio::spawn({
        let config = config.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) = watcher::run_watcher(config.watcher, watcher_tx, shutdown_rx).await {
                error!(error = %e, "watcher failed");
            }
        }
    });

    let scanner_handle = tokio::spawn({
        let config = config.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) = scanner::run_scanner(config.scanner, watcher_rx, scanner_tx, shutdown_rx).await {
                error!(error = %e, "scanner failed");
            }
        }
    });

    let metadata_handle = tokio::spawn({
        let config = config.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) = metadata::run_metadata(config.organizer, scanner_rx, metadata_tx, shutdown_rx).await {
                error!(error = %e, "metadata failed");
            }
        }
    });

    let organizer_handle = tokio::spawn({
        let config = config.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) = organizer::run_organizer(config.organizer, metadata_rx, organizer_tx, shutdown_rx).await {
                error!(error = %e, "organizer failed");
            }
        }
    });

    let nextcloud_handle = tokio::spawn({
        let config = config.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) = nextcloud::run_nextcloud(config.nextcloud, organizer_rx, nextcloud_tx, shutdown_rx).await {
                error!(error = %e, "nextcloud failed");
            }
        }
    });

    info!("watching for file events");

    loop {
        tokio::select! {
            Some(event) = output_rx.recv() => {
                match event {
                    FileEvent::Organized { old_path, new_path } => {
                        info!(
                            from = %old_path.display(),
                            to = %new_path.display(),
                            "file organized"
                        );
                    }
                    FileEvent::Failed { path, error } => {
                        warn!(path = %path.display(), error, "processing failed");
                    }
                    _ => {}
                }
            }

            _ = tokio::signal::ctrl_c() => {
                info!("received shutdown signal, draining pipeline");
                shutdown_tx.send(()).ok();
                break;
            }
        }
    }

    let shutdown_timeout = std::time::Duration::from_secs(30);
    let all_actors = async {
        let _ = watcher_handle.await;
        let _ = scanner_handle.await;
        let _ = metadata_handle.await;
        let _ = organizer_handle.await;
        let _ = nextcloud_handle.await;
    };

    if tokio::time::timeout(shutdown_timeout, all_actors).await.is_err() {
        warn!("shutdown timed out after 30s, forcing exit");
    } else {
        info!("shutdown complete");
    }

    Ok(())
}
