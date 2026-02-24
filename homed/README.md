# homed

A file watcher and organizer daemon for home server. Monitors directories for new files and processes them through two independent pipelines: one for photos and one for media downloads.

## What It Does

### Photos Pipeline

Watches a directory (e.g. Nextcloud uploads) for new photos and videos, then:

1. **Watcher** detects new files after a configurable debounce period, ignoring incomplete downloads (`.!qb`, `.part`)
2. **Metadata** classifies files as photo or video based on extension, extracts the best available datetime from EXIF data, filename patterns (`IMG_20260211_143022.jpg`), or file modification time
3. **Organizer** moves files into a date-based directory structure:
   ```
   Photos/2026/2026-02/IMG_20260211_143022.jpg
   ```
   Handles filename collisions by appending `_1`, `_2`, etc. Optionally sets file ownership (e.g. `www-data` for Nextcloud)
4. **Nextcloud** triggers `occ files:scan` via `docker exec` so Nextcloud picks up the new files without a full rescan

### Media Pipeline

Post-import guard on library directories (Movies/, TV/). Radarr/Sonarr handle importing from downloads — homed watches the library directories and cleans up after them:

1. **Watcher** detects new files after debounce
2. **Scanner** processes each file:
   - Deletes junk files (`.nfo`, `.txt`, cover images) that arrived with the import
   - Quarantines executables and files with mismatched headers
   - Validates allowed extensions and file sizes
   - Removes empty directories left behind after cleanup

### Alerts

Sends push notifications via [ntfy](https://ntfy.sh) for:
- **Quarantined** events: a suspicious file was moved to quarantine
- **Organized** events: a photo was sorted into Nextcloud

## Architecture

```
                          Photos Pipeline
                ┌──────┐  ┌──────────┐  ┌───────────┐  ┌───────────┐
  filesystem ──>│Watch │─>│ Metadata │─>│ Organizer │─>│ Nextcloud │──┐
                └──────┘  └──────────┘  └───────────┘  └───────────┘  │
                                                                       ├─> Output (log + alert)
                          Media Pipeline                               │
                ┌──────┐  ┌─────────┐                                │
  filesystem ──>│Watch │─>│ Scanner │──────────────────────────────────┘
                └──────┘  └─────────┘
```

Each stage is a Tokio task connected by mpsc channels. Events flow through the pipeline and any stage can emit `Failed` events which propagate to the output for logging and alerting. Graceful shutdown is handled via a broadcast channel on `SIGINT`.

## File Security Checks

The scanner runs multiple validation layers on incoming files:

- **Extension whitelist**: only configured extensions pass through
- **Executable blocking**: rejects `.exe`, `.bat`, `.sh`, `.py`, `.jar`, and other executable extensions
- **Minimum size check**: catches suspiciously small video files (< 1KB)
- **Magic byte validation**: reads file headers and verifies they match the claimed extension. Catches PE executables disguised as `.mkv`, ELF binaries disguised as `.mp4`, etc.
- **Subtitle validation**: verifies `.srt`/`.ass` files are valid UTF-8 text

## Configuration

All paths, credentials, and behavior are configured in `config.toml`. Copy the example and edit:

```bash
cp config.example.toml /opt/homed/config.toml
```

### Photos Pipeline

| Key | Description |
|-----|-------------|
| `photos.watcher.paths` | Directories to watch for new uploads |
| `photos.watcher.debounce_ms` | Milliseconds of silence before processing a file (100-60000) |
| `photos.organizer.enabled` | Enable/disable photo organization |
| `photos.organizer.photos_dir` | Root directory for organized photos |
| `photos.organizer.photo_prefix` | Filename prefix for photos (e.g. `IMG`) |
| `photos.organizer.video_prefix` | Filename prefix for videos (e.g. `VID`) |
| `photos.organizer.photo_extensions` | File extensions to treat as photos |
| `photos.organizer.video_extensions` | File extensions to treat as videos |
| `photos.organizer.file_owner` | Optional: set file owner after move |
| `photos.organizer.file_group` | Optional: set file group after move |
| `photos.nextcloud.enabled` | Enable/disable Nextcloud scan triggers |
| `photos.nextcloud.container_name` | Docker container name for Nextcloud |
| `photos.nextcloud.username` | Nextcloud username |
| `photos.nextcloud.data_dir` | Host path to Nextcloud data directory |
| `photos.nextcloud.internal_prefix` | Nextcloud internal path prefix |

### Media Pipeline

| Key | Description |
|-----|-------------|
| `media.watcher.paths` | Library directories to watch (e.g. Movies/, TV/) |
| `media.watcher.debounce_ms` | Debounce period in milliseconds |
| `media.watcher.ignore_extensions` | Extensions to skip (e.g. `!qb`, `part`) |
| `media.scanner.quarantine_dir` | Where suspicious files are moved |
| `media.scanner.allowed_extensions` | Whitelist of allowed file extensions |
| `media.scanner.block_executables` | Block files with executable extensions |
| `media.scanner.delete_junk` | Delete files with junk extensions |
| `media.scanner.junk_extensions` | Extensions to treat as junk (e.g. `nfo`, `txt`, `jpg`) |

### Alerts

| Key | Description |
|-----|-------------|
| `alerts.enabled` | Enable/disable push notifications |
| `alerts.url` | ntfy server URL (e.g. `https://ntfy.example.com`) |
| `alerts.topic` | ntfy topic to publish to |
| `alerts.token` | Bearer token for ntfy authentication |

## Installation

### Fresh Machine

The `home/setup.sh` script handles installation automatically: downloads the latest release binary, copies the example config, installs the systemd service, and enables it.

After running setup, edit `/opt/homed/config.toml` with your actual paths and credentials, then start:

```bash
sudo systemctl start homed
```

### Updating

After pushing changes to `main`, the GitHub Actions workflow builds a new release. Deploy on the server:

```bash
cd ~/Homeserver/homed
bash deploy.sh
```

Or manually:

```bash
curl -fL -o /tmp/homed https://github.com/koinsaari/homeserver/releases/latest/download/homed
sudo systemctl stop homed
sudo install -m 755 /tmp/homed /opt/homed/homed
sudo systemctl start homed
```

### Logs

```bash
journalctl -u homed -f
```

## Building From Source

Requires Rust 1.85+:

```bash
cd homed
cargo build --release
```

The binary is at `target/release/homed`. For a static binary (no glibc dependency):

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```
