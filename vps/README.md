# VPS Deployment

NetBird, ntfy, and MollySocket stack for public VPS deployment.

## Prerequisites

- VPS with 2GB+ RAM (tested on Hetzner CX21)
- Domain name with DNS access
- SSH access to VPS

## Files

- `docker-compose.yml` - Service definitions (NetBird, ntfy, MollySocket, Caddy)
- `Caddyfile` - Reverse proxy config with automatic HTTPS
- `.env.example` - Environment variables template
- `setup.sh` - VPS hardening and Docker installation script

## Deployment Steps

### 1. Configure DNS

Point these A records to your VPS IP:
```
netbird.yourdomain.com  → VPS_IP
ntfy.yourdomain.com     → VPS_IP
molly.yourdomain.com    → VPS_IP
```

Wait 5-10 minutes for DNS propagation.

### 2. Run Setup Script

SSH to VPS and run:
```bash
bash setup.sh
```

This installs Docker, configures UFW firewall, and optionally creates a non-root user.

### 3. Deploy Services

Copy files to VPS:
```bash
# On local machine:
scp -r vps/* user@vps-ip:/opt/homeserver/
```

On VPS:
```bash
cd /opt/homeserver

# Create .env from template
cp .env.example .env

# Generate secrets
NETBIRD_ENCRYPTION_KEY=$(openssl rand -base64 32)
NETBIRD_RELAY_SECRET=$(openssl rand -base64 32)
MOLLY_VAPID_PRIVKEY=$(docker run --rm ghcr.io/mollyim/mollysocket:latest vapid gen)

# Edit .env and fill in:
# - DOMAIN
# - VPS_IP
# - ACME_EMAIL
# - Generated secrets above

# Start services
docker-compose up -d

# Watch logs
docker-compose logs -f
```

### 4. Post-Deployment Configuration

#### Create ntfy Admin User

**IMPORTANT:** ntfy requires authentication. Create an admin user:

```bash
docker exec -it ntfy ntfy user add --role=admin your_username
```

You'll be prompted for a password. Use this username/password for ntfy clients.

#### Create NetBird Account

1. Visit `https://netbird.yourdomain.com`
2. Click "Setup" (first-time only)
3. Create your admin account
4. Enroll devices via setup keys

## Verification

Check all services are healthy:
```bash
docker-compose ps
```

All containers should show `healthy` status.

Test endpoints:
```bash
curl https://netbird.yourdomain.com
curl https://ntfy.yourdomain.com
curl https://molly.yourdomain.com
```

## Resource Usage

| Service | RAM | Purpose |
|---------|-----|---------|
| netbird-management | 1GB | Management API + embedded IdP |
| netbird-signal | 256MB | P2P signaling |
| netbird-relay | 256MB | STUN/TURN relay |
| netbird-dashboard | 128MB | Web UI |
| ntfy | 256MB | Push notifications |
| mollysocket | 128MB | Signal UnifiedPush |
| caddy | 128MB | Reverse proxy + TLS |
| **Total** | ~2.15GB | Works on budget VPS |

## Firewall

UFW rules (configured by setup.sh):
- 22/tcp - SSH
- 80/tcp - HTTP (ACME challenges)
- 443/tcp - HTTPS
- 3478/tcp+udp - NetBird relay

## Backup

Critical data volumes:
- `netbird_data` - NetBird database and IdP data
- `ntfy_data` - ntfy user auth database

Use VPS snapshots (if available) or manual backup:
```bash
docker run --rm -v netbird_data:/data -v $(pwd):/backup alpine tar czf /backup/netbird_data.tar.gz /data
docker run --rm -v ntfy_data:/data -v $(pwd):/backup alpine tar czf /backup/ntfy_data.tar.gz /data
```

## Troubleshooting

**Caddy fails to get certificates:**
- Verify DNS is pointing to VPS IP
- Check ports 80/443 are not blocked
- Check logs: `docker logs caddy`

**NetBird management unhealthy:**
- Check encryption key is set in .env
- Verify relay auth secret matches
- Check logs: `docker logs netbird-management`

**Can't send ntfy notifications:**
- Ensure you created an admin user (see step 4)
- Verify authentication in ntfy client
- Check logs: `docker logs ntfy`
