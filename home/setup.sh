#!/bin/bash
set -euo pipefail

if [ "$EUID" -ne 0 ]; then
    echo "Please run with sudo: sudo ./setup.sh"
    exit 1
fi

REAL_USER=${SUDO_USER:-$USER}

if [ "$REAL_USER" = "root" ]; then
    echo "Error: Run this script as a regular user with sudo, not as root directly."
    exit 1
fi

echo "Home Server Setup"
echo "================================"
echo "This script will configure:"
echo "  - ZRAM (4GB compressed swap)"
echo "  - NetBird VPN client"
echo "  - Docker"
echo "  - UFW firewall"
echo "  - Traefik directories"
echo "  - Media library structure (/mnt/media)"
echo ""
read -p "Continue with installation? (y/n) " -r
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    exit 0
fi

echo ""
echo "[1/8] Updating system..."
apt-get update
apt-get upgrade -y
apt-get install -y curl ufw fail2ban unattended-upgrades git htop ncdu jq rsync tree smartmontools parted zram-tools

echo ""
echo "[2/8] Intel Quick Sync GPU setup..."
INSTALL_GPU="n"
if [ -d /dev/dri ]; then
    echo "Detected /dev/dri - Intel GPU may be available"
    read -p "Install Intel Quick Sync drivers for Jellyfin transcoding? (y/n) " -r INSTALL_GPU
fi

if [[ $INSTALL_GPU =~ ^[Yy]$ ]]; then
    echo "Installing Intel Quick Sync drivers (i965 for Broadwell support)..."
    if apt-get install -y intel-gpu-tools vainfo i965-va-driver intel-media-va-driver 2>/dev/null; then
        VIDEO_GID=$(getent group video | cut -d: -f3)
        RENDER_GID=$(getent group render | cut -d: -f3)
        echo "Installed: i965-va-driver (legacy) + intel-media-va-driver (modern)"
        echo "Detected video group: $VIDEO_GID, render group: $RENDER_GID"
    else
        echo "Warning: GPU driver installation failed, continuing without Quick Sync"
        VIDEO_GID=""
        RENDER_GID=""
    fi
else
    VIDEO_GID=""
    RENDER_GID=""
fi

echo ""
echo "[3/8] Configuring ZRAM (4GB compressed swap)..."
cat > /etc/default/zramswap << 'EOF'
ALGO=zstd
PERCENT=50
PRIORITY=100
EOF

cat > /etc/sysctl.d/99-zram.conf << 'EOF'
vm.swappiness=10
vm.vfs_cache_pressure=50
EOF

sysctl -p /etc/sysctl.d/99-zram.conf

systemctl restart zramswap

if [ -f /swapfile ]; then
    echo "Disabling old swap file..."
    swapoff /swapfile 2>/dev/null || true
    sed -i '/\/swapfile/d' /etc/fstab
    rm -f /swapfile
fi

echo ""
echo "[4/8] Configuring SSH security..."
if [ -f /etc/ssh/sshd_config ]; then
    cp /etc/ssh/sshd_config /etc/ssh/sshd_config.bak
    sed -i 's/^#\?PermitRootLogin.*/PermitRootLogin no/' /etc/ssh/sshd_config
    sed -i 's/^#\?PermitEmptyPasswords.*/PermitEmptyPasswords no/' /etc/ssh/sshd_config
    systemctl restart ssh
fi
systemctl enable fail2ban
systemctl start fail2ban

echo ""
echo "[5/8] Disabling laptop lid suspend..."
sed -i 's/^#\?HandleLidSwitch=.*/HandleLidSwitch=ignore/' /etc/systemd/logind.conf
sed -i 's/^#\?HandleLidSwitchDocked=.*/HandleLidSwitchDocked=ignore/' /etc/systemd/logind.conf
systemctl restart systemd-logind

echo ""
echo "[6/8] Installing Docker..."
if ! command -v docker &> /dev/null; then
    curl -fsSL https://get.docker.com | sh
fi
usermod -aG docker "$REAL_USER"

echo ""
echo "[7/8] Installing NetBird..."
if ! command -v netbird &> /dev/null; then
    curl -fsSL https://pkgs.netbird.io/install.sh | sh
fi

echo ""
if netbird status 2>/dev/null | grep -q "Connected"; then
    echo "NetBird is already connected."
else
    read -p "Configure NetBird VPN now? (y/n) " -r SETUP_NETBIRD
    if [[ $SETUP_NETBIRD =~ ^[Yy]$ ]]; then
        echo ""
        echo "================================================"
        echo "NetBird Authentication"
        echo "================================================"
        echo "Run this in another terminal:"
        echo ""
        echo "  sudo netbird up"
        echo ""
        echo "If it gets stuck, press Ctrl+C here to skip."
        read -t 300 -p "Press ENTER when connected (5 min timeout)..." || echo "Timeout - skipping NetBird"

        if netbird status 2>/dev/null | grep -q "Connected"; then
            echo "NetBird connected successfully!"
        else
            echo "NetBird not connected. You can set it up later with: sudo netbird up"
        fi
    else
        echo "Skipping NetBird setup. Run 'sudo netbird up' manually when ready."
    fi
fi

echo ""
echo "[8/8] Configuring UFW firewall (zero-trust)..."
if ! ufw status | grep -qw "active"; then
    read -p "Enter your LAN subnet (e.g., 192.168.1.0/24): " LAN_SUBNET
    LAN_SUBNET=${LAN_SUBNET:-192.168.1.0/24}

    ufw --force reset
    ufw default deny incoming
    ufw default allow outgoing

    ufw allow from "$LAN_SUBNET" to any port 22 proto tcp comment 'SSH from LAN'

    ufw allow in on wt0 comment 'NetBird mesh traffic'
    ufw allow from 100.64.0.0/10 to any port 80 comment 'NetBird HTTP'
    ufw allow from 100.64.0.0/10 to any port 443 comment 'NetBird HTTPS'

    echo "y" | ufw enable
else
    echo "UFW is already active. Skipping firewall reset."
fi

echo ""
echo "Setting up Traefik directories..."
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

mkdir -p "$SCRIPT_DIR/traefik/dynamic"
chown -R "$REAL_USER:$REAL_USER" "$SCRIPT_DIR/traefik"

mkdir -p /mnt/hot/nextcloud-data
chown -R "$REAL_USER:$REAL_USER" /mnt/hot/nextcloud-data 2>/dev/null || true

mkdir -p /mnt/media/downloads/complete/{movies,tv}
mkdir -p /mnt/media/downloads/ready/{movies,tv}
mkdir -p /mnt/media/{Movies,TV}
chown -R "$REAL_USER:$REAL_USER" /mnt/media 2>/dev/null || true

echo ""
echo "Generating .env file..."

if [ -f "$SCRIPT_DIR/.env" ]; then
    echo "Existing .env found, loading values."
    cp "$SCRIPT_DIR/.env" "$SCRIPT_DIR/.env.bak"
    set -a
    # shellcheck disable=SC1090
    source "$SCRIPT_DIR/.env"
    set +a
fi

[ -z "${DOMAIN:-}" ]     && read -p "Base domain (e.g., example.com): " DOMAIN
[ -z "${ACME_EMAIL:-}" ] && read -p "ACME email for Let's Encrypt: " ACME_EMAIL
[ -z "${TZ:-}" ]         && read -p "Timezone (default: Europe/Helsinki): " TZ
TZ=${TZ:-Europe/Helsinki}

echo ""
echo "DNS Provider for ACME DNS-01 challenge"
echo "See: https://go-acme.github.io/lego/dns/"
[ -z "${DNS_PROVIDER:-}" ] && read -p "DNS provider (e.g., njalla, cloudflare): " DNS_PROVIDER

DNS_TOKEN_VAR=""
case "$DNS_PROVIDER" in
    cloudflare) DNS_TOKEN_VAR="CLOUDFLARE_DNS_API_TOKEN" ;;
    hetzner)    DNS_TOKEN_VAR="HETZNER_API_TOKEN" ;;
    njalla)     DNS_TOKEN_VAR="NJALLA_TOKEN" ;;
    porkbun)    DNS_TOKEN_VAR="PORKBUN_API_KEY" ;;
    route53)    DNS_TOKEN_VAR="AWS_ACCESS_KEY_ID" ;;
    *)          DNS_TOKEN_VAR="${DNS_PROVIDER^^}_TOKEN" ;;
esac

DNS_TOKEN="${!DNS_TOKEN_VAR:-}"
[ -z "$DNS_TOKEN" ] && read -p "DNS API token ($DNS_TOKEN_VAR): " DNS_TOKEN

echo ""
[ -z "${POSTGRES_PASSWORD:-}" ] && read -p "PostgreSQL password: " POSTGRES_PASSWORD
[ -z "${NEXTCLOUD_ADMIN_USER:-}" ] && read -p "Nextcloud admin username: " NEXTCLOUD_ADMIN_USER
if [ -z "${NEXTCLOUD_ADMIN_PASSWORD:-}" ]; then
    read -s -p "Nextcloud admin password: " NEXTCLOUD_ADMIN_PASSWORD
    echo ""
fi

echo ""
echo "VPN for qBittorrent (routes torrent traffic through VPN)"
[ -z "${VPN_PROVIDER:-}" ] && read -p "VPN provider (default: protonvpn): " VPN_PROVIDER
VPN_PROVIDER=${VPN_PROVIDER:-protonvpn}
[ -z "${VPN_TYPE:-}" ] && read -p "VPN type (default: wireguard): " VPN_TYPE
VPN_TYPE=${VPN_TYPE:-wireguard}
[ -z "${VPN_PRIVATE_KEY:-}" ] && read -p "WireGuard private key: " VPN_PRIVATE_KEY
[ -z "${VPN_SERVER_COUNTRIES:-}" ] && read -p "VPN server country (default: Netherlands): " VPN_SERVER_COUNTRIES
VPN_SERVER_COUNTRIES=${VPN_SERVER_COUNTRIES:-Netherlands}

echo ""
echo "NetBird VPN binding"
echo "Run 'ip addr show wt0 | grep inet' to find your NetBird IP"
[ -z "${NETBIRD_IP:-}" ] && read -p "NetBird IP (e.g., 100.64.x.x): " NETBIRD_IP

VAULTWARDEN_ADMIN_PLAIN=""
if [ -z "${ADMIN_TOKEN:-}" ]; then
    echo ""
    echo "Generating Vaultwarden admin token (requires Docker)..."
    VAULTWARDEN_ADMIN_PLAIN=$(openssl rand -base64 48 | tr -d '\n')
    ADMIN_TOKEN=$(printf '%s' "$VAULTWARDEN_ADMIN_PLAIN" \
        | docker run --rm -i vaultwarden/server /vaultwarden hash 2>/dev/null \
        | grep -o '\$argon2.*')

    if [ -z "$ADMIN_TOKEN" ]; then
        echo "Warning: Could not generate Argon2 hash (Docker may not be running yet)."
        echo "Falling back to plain-text token. Replace ADMIN_TOKEN in .env later with:"
        echo "  docker run --rm -it vaultwarden/server /vaultwarden hash"
        ADMIN_TOKEN="$VAULTWARDEN_ADMIN_PLAIN"
    fi
fi

REAL_UID=$(id -u "$REAL_USER")
REAL_GID=$(id -g "$REAL_USER")

cat > "$SCRIPT_DIR/.env" << EOF
DOMAIN=$DOMAIN
ACME_EMAIL=$ACME_EMAIL
TZ=$TZ

PUID=$REAL_UID
PGID=$REAL_GID
VIDEO_GID=$VIDEO_GID
RENDER_GID=$RENDER_GID

DNS_PROVIDER=$DNS_PROVIDER
$DNS_TOKEN_VAR=$DNS_TOKEN

POSTGRES_PASSWORD=$POSTGRES_PASSWORD

NEXTCLOUD_ADMIN_USER=$NEXTCLOUD_ADMIN_USER
NEXTCLOUD_ADMIN_PASSWORD=$NEXTCLOUD_ADMIN_PASSWORD

VPN_PROVIDER=$VPN_PROVIDER
VPN_TYPE=$VPN_TYPE
VPN_PRIVATE_KEY=$VPN_PRIVATE_KEY
VPN_SERVER_COUNTRIES=$VPN_SERVER_COUNTRIES

NETBIRD_IP=$NETBIRD_IP
EOF

printf "ADMIN_TOKEN='%s'\n" "$ADMIN_TOKEN" >> "$SCRIPT_DIR/.env"

chmod 600 "$SCRIPT_DIR/.env"
chown "$REAL_USER:$REAL_USER" "$SCRIPT_DIR/.env"

echo ""
echo "Generating Traefik configuration..."
cat > "$SCRIPT_DIR/traefik/traefik.yml" << EOF
api:
  dashboard: true

ping: {}

log:
  level: INFO

entryPoints:
  web:
    address: ":80"
    http:
      redirections:
        entryPoint:
          to: websecure
          scheme: https
  websecure:
    address: ":443"

providers:
  docker:
    endpoint: "unix:///var/run/docker.sock"
    exposedByDefault: false
    network: home_internal
  file:
    directory: /etc/traefik/dynamic
    watch: true

certificatesResolvers:
  letsencrypt:
    acme:
      email: "$ACME_EMAIL"
      storage: /certs/acme.json
      dnsChallenge:
        provider: "$DNS_PROVIDER"
        propagation:
          delayBeforeChecks: 30
        resolvers:
          - "1.1.1.1:53"
          - "8.8.8.8:53"
EOF

chown "$REAL_USER:$REAL_USER" "$SCRIPT_DIR/traefik/traefik.yml"

echo ""
echo "================================================"
echo "Setup Complete!"
echo "================================================"
echo ""
echo "IMPORTANT: You must log out and log back in"
echo "for Docker group changes to take effect."
echo ""
echo "Next steps:"
echo "  1. Log out and log back in"
echo "  2. Verify NetBird: netbird status"
echo "  3. Start services: docker compose up -d"
echo ""
echo "Services will be available at:"
echo "  - https://nextcloud.$DOMAIN"
echo "  - https://jellyfin.$DOMAIN"
echo "  - https://vault.$DOMAIN"
echo "  - https://qbit.$DOMAIN"
echo "  - https://prowlarr.$DOMAIN"
echo "  - https://sonarr.$DOMAIN"
echo "  - https://radarr.$DOMAIN"
echo "  - https://bazarr.$DOMAIN"
echo ""
echo "Credentials saved in: $SCRIPT_DIR/.env"
if [ -n "$VAULTWARDEN_ADMIN_PLAIN" ]; then
    echo ""
    echo "Vaultwarden admin password (save this, it is not stored anywhere):"
    echo "  $VAULTWARDEN_ADMIN_PLAIN"
    echo "  Admin panel: https://vault.$DOMAIN/admin"
fi
echo "================================================"
