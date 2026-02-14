# VPS Stack: NetBird, ntfy, MollySocket

Hardened deployment for a private communication stack using Caddy for automatic TLS.

## Deployment

### 1. DNS Configuration
Point A records to your VPS IP:
- `netbird.yourdomain.com`
- `ntfy.yourdomain.com`
- `molly.yourdomain.com`

### 2. Initial Setup
Run as root or with sudo:
```bash
bash setup.sh
```

This script performs:
- System updates and package installation
- Non-root user creation (optional)
- Docker installation
- UFW firewall configuration (ports: 22, 80, 443, 3478)
- SSH hardening (disables root login if non-root user created)
- Docker network (`proxy-net`) creation
- Data directory initialization with proper permissions

**Important**: If running as non-root user, you must log out and back in after setup for docker group changes to take effect.

### 3. NetBird Installation
```bash
curl -fsSL https://github.com/netbirdio/netbird/releases/latest/download/getting-started.sh | bash
```
Select **[4] External Caddy** and specify `proxy-net` as the network.

### 4. Environment Configuration
Copy and configure environment variables:
```bash
cp .env.example .env
```

Required variables:
- `DOMAIN`: Your base domain (e.g., `example.com`)
- `ACME_EMAIL`: Email for Let's Encrypt certificates
- `MOLLY_VAPID_PRIVKEY`: Generate in next step

### 5. VAPID Keys
If you don't already have a VAPID key from an earlier deployment, generate one:
```bash
docker run --rm ghcr.io/mollyim/mollysocket:latest mollysocket-tools generate-vapid
```
Add the private key to `MOLLY_VAPID_PRIVKEY` in `.env`.

### 6. Start Services
```bash
docker compose up -d
```

### 7. Configure ntfy Authentication
Create an admin user (required since default access is deny-all):
```bash
docker exec -it ntfy ntfy user add --role=admin username
```

### 8. Enable UnifiedPush Support
To allow MollySocket to send notifications to your phone while keeping the rest of the server private, grant write-only access to UnifiedPush topics:

```bash
docker exec -it ntfy ntfy access '*' 'up*' write-only
```

## Security Features

- **Firewall**: UFW blocks all incoming except SSH (22), HTTP (80), HTTPS (443), and NetBird relay (3478)
- **Containers**: All run with 256MB memory limits and `no-new-privileges:true`
- **User Isolation**: ntfy runs as non-root user (1000:1000)
- **Capability Dropping**: Caddy drops all capabilities except `NET_BIND_SERVICE`
- **Authentication**: ntfy requires user authentication (deny-all default)
- **SSH Hardening**: Root login disabled when non-root user is created
- **Data Persistence**: All volumes use local bind mounts for direct filesystem access

**Note on MollySocket**: Currently runs as container default user (root). It seems the image has issues with SQLite file permissions when attempting to run as a non-root user with `user: "1000:1000"`, likely due to directory ownership requirements for database initialization.

## Stack Components

| Service | Port | Domain | Purpose |
|---------|------|--------|---------|
| Caddy | 80, 443 | All | Reverse proxy with automatic TLS |
| NetBird | - | netbird.* | VPN/mesh network |
| ntfy | 8082 | ntfy.* | Push notification server |
| MollySocket | 8091 | molly.* | UnifiedPush distributor for Molly |
