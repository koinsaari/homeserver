mod config;

use config::Config;

fn main() -> anyhow::Result<()> {
    println!("🏠 homed - home server daemon");

    let config_path = "config.toml";

    match Config::load(config_path) {
        Ok(config) => {
            println!("✅ Configuration loaded successfully!");
            println!("   Watching {} paths", config.watcher.paths.len());
            println!("   Debounce: {}ms", config.watcher.debounce_ms);
        }
        Err(e) => {
            eprintln!("❌ Failed to load config: {}", e);
            eprintln!("   Create a config.toml file to get started");
        }
    }

    Ok(())
}
