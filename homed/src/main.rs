mod config;
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

    let (tx, mut rx) = mpsc::channel(100);

    tokio::spawn(async move {
        if let Err(e) = watcher::run_watcher(config.watcher, tx).await {
            eprintln!("Watcher error: {}", e);
        }
    });

    println!("\n👀 Watching for file events... (Ctrl+C to stop)\n");

    while let Some(event) = rx.recv().await {
        match event {
            FileEvent::Detected { path, size } => {
                println!("📁 Detected: {} ({} bytes)", path.display(), size);
            }
            _ => {}
        }
    }

    Ok(())
}
