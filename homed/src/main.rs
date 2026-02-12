mod config;
mod metadata;
mod organizer;
mod watcher;
mod scanner;

use config::Config;
use tokio::sync::mpsc;
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

    let (watcher_tx, watcher_rx) = mpsc::channel(100);
    let (scanner_tx, scanner_rx) = mpsc::channel(100);
    let (metadata_tx, metadata_rx) = mpsc::channel(100);
    let (organizer_tx, mut output_rx) = mpsc::channel(100);

    tokio::spawn({
        let config = config.clone();
        async move {
            if let Err(e) = watcher::run_watcher(config.watcher, watcher_tx).await {
                eprintln!("Watcher error: {}", e);
            }
        }
    });

    tokio::spawn({
        let config = config.clone();
        async move {
            if let Err(e) = scanner::run_scanner(config.scanner, watcher_rx, scanner_tx).await {
                eprintln!("Scanner error: {}", e);
            }
        }
    });

    tokio::spawn({
        let config = config.clone();
        async move {
            if let Err(e) = metadata::run_metadata(config.organizer, scanner_rx, metadata_tx).await {
                eprintln!("Metadata error: {}", e);
            }
        }
    });

    tokio::spawn({
        let config = config.clone();
        async move {
            if let Err(e) = organizer::run_organizer(config.organizer, metadata_rx, organizer_tx).await {
                eprintln!("Organizer error: {}", e);
            }
        }
    });

    println!("\n👀 Watching for file events... (Ctrl+C to stop)\n");

    while let Some(event) = output_rx.recv().await {
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

    Ok(())
}
