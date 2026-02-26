# Home Server Stack

Private services accessible only via NetBird mesh, with Let's Encrypt wildcard certificates via DNS-01.

## TLS Strategy

Uses a wildcard certificate (`*.yourdomain.com`) via DNS-01 challenge:
- **Privacy**: Only `*.yourdomain.com` appears in Certificate Transparency logs
- **No inbound ports**: DNS-01 is outbound-only, home server stays airgapped
- **Zero client config**: Friends see green padlock, no CA sideloading required

## Architecture

```
Internet ──X──┐
              │ (blocked by NAT)
              ▼
        ┌─────────────┐
        │ Home Server │◀─── NetBird Mesh ───▶ Phones, Laptops, TVs
        └─────────────┘
              │
    ┌─────────┴─────────┐
    ▼                   ▼
Traefik             Services
(TLS + routing)     (Nextcloud, Jellyfin, *arr, etc.)
```

## Services

| Service | Purpose |
|---------|---------|
| **Traefik** | Reverse proxy with automatic wildcard TLS via DNS-01 |
| **Nextcloud** | Cloud storage and photo backup (PostgreSQL backend) |
| **Jellyfin** | Media server with Intel Quick Sync hardware transcoding |
| **Vaultwarden** | Bitwarden-compatible password manager |
| **qBittorrent** | Torrent client, all traffic routed through Gluetun VPN |
| **Sonarr / Radarr** | TV and movie automation |
| **Prowlarr** | Indexer manager for Sonarr/Radarr |
| **Bazarr** | Subtitle automation |
| **homed** | Native daemon — file watcher, media scanner, photo organizer, ntfy alerts |

## Security

- **No WAN exposure**: Home server behind NAT, no port forwarding, NetBird-only access
- **TLS**: Wildcard cert via DNS-01, no subdomain metadata in CT logs
- **Containers**: Memory limits, `no-new-privileges:true`, capabilities dropped to minimum required
- **VPN-tunneled torrents**: qBittorrent traffic routed through WireGuard via Gluetun
- **Media scanning**: homed validates file types by magic bytes, blocks disguised executables
- **Health checks**: All services monitored with restart policies

## DNS Resolution

Configure NetBird DNS to resolve subdomains to the home server's NetBird IP:
- `*.yourdomain.com` → `100.64.0.x` (home server's NetBird IP)
