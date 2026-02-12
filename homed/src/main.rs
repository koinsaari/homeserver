mod config;
mod metadata;
mod nextcloud;
mod organizer;
mod scanner;
mod watcher;

use config::Config;
use tokio::sync::{broadcast, mpsc};
use watcher::FileEvent;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🏠 homed - home server daemon");

    let config = Config::load("config.toml")?;
    println!("✅ Configuration loaded successfully!");
    println!("   Watching {} paths", config.watcher.paths.len());
    println!("   Debounce: {}ms", config.watcher.debounce_ms);
    println!("   Scanner: {}", if config.scanner.enabled { "enabled" } else { "disabled" });
    println!("   Organizer: {}", if config.organizer.enabled { "enabled" } else { "disabled" });
    println!("   Nextcloud: {}", if config.nextcloud.enabled { "enabled" } else { "disabled" });

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
                eprintln!("Watcher error: {}", e);
            }
        }
    });

    let scanner_handle = tokio::spawn({
        let config = config.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) = scanner::run_scanner(config.scanner, watcher_rx, scanner_tx, shutdown_rx).await {
                eprintln!("Scanner error: {}", e);
            }
        }
    });

    let metadata_handle = tokio::spawn({
        let config = config.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) = metadata::run_metadata(config.organizer, scanner_rx, metadata_tx, shutdown_rx).await {
                eprintln!("Metadata error: {}", e);
            }
        }
    });

    let organizer_handle = tokio::spawn({
        let config = config.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) = organizer::run_organizer(config.organizer, metadata_rx, organizer_tx, shutdown_rx).await {
                eprintln!("Organizer error: {}", e);
            }
        }
    });

    let nextcloud_handle = tokio::spawn({
        let config = config.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        async move {
            if let Err(e) = nextcloud::run_nextcloud(config.nextcloud, organizer_rx, nextcloud_tx, shutdown_rx).await {
                eprintln!("Nextcloud error: {}", e);
            }
        }
    });

    println!("\n👀 Watching for file events... (Ctrl+C to stop)\n");

    loop {
        tokio::select! {
            Some(event) = output_rx.recv() => {
                match event {
                    FileEvent::Organized { old_path, new_path } => {
                        println!("📦 Organized: {} → {}", old_path.display(), new_path.display());
                    }
                    FileEvent::Failed { path, error } => {
                        println!("❌ Failed: {} - {}", path.display(), error);
                    }
                    _ => {}
                }
            }

            _ = tokio::signal::ctrl_c() => {
                eprintln!("\n🛑 Received shutdown signal, draining pipeline...");
                shutdown_tx.send(()).ok();
                break;
            }
        }
    }

    // Wait for all actors to finish work
    let shutdown_timeout = std::time::Duration::from_secs(30);
    let all_actors = async {
        let _ = watcher_handle.await;
        let _ = scanner_handle.await;
        let _ = metadata_handle.await;
        let _ = organizer_handle.await;
        let _ = nextcloud_handle.await;
    };

    if tokio::time::timeout(shutdown_timeout, all_actors).await.is_err() {
        eprintln!("⚠️  Shutdown timed out after 30s, forcing exit");
    } else {
        eprintln!("✅ Shutdown complete");
    }

    Ok(())
}
