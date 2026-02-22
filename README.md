# Simple Home Server

My self-hosted setup running on an old laptop.

## What's Running

### Home Server

- **Nextcloud** — cloud storage and photo backup, running in Docker behind Traefik
- **Jellyfin** — media server with Intel Quick Sync hardware transcoding
- **qBittorrent** — torrent client with all traffic routed through WireGuard VPN
- **Sonarr / Radarr / Prowlarr / Bazarr** — media automation: indexing, downloading, renaming, subtitles
- **homed** — Rust daemon that watches directories and runs two pipelines:
  - Photos: EXIF extraction → date-based renaming → Nextcloud `occ files:scan`
  - Media: extension whitelist → magic byte validation → hardlink to import directory

### VPS

- **ntfy** — self-hosted push notification server, auth-gated with bearer tokens
- **MollySocket** — UnifiedPush provider bridging Signal notifications to ntfy
- **Caddy** — reverse proxy handling TLS termination for all services

## Networking

The home server has no open ports on the WAN. All remote access goes through NetBird, a WireGuard-based mesh VPN. The VPS exposes only ports 80 and 443 through Caddy.

## License

MIT
