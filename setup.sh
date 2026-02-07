#!/bin/bash

set -euo pipefail

if [ "$EUID" -ne 0 ]; then
    echo "Please run with sudo: sudo ./setup.sh"
    exit 1
fi

if [ -z "${SUDO_USER:-}" ]; then
    echo "Please run with sudo, not as root directly"
    exit 1
fi

REAL_USER=$SUDO_USER

echo "Server Setup"
echo "================================"
echo "Installing Docker, Tailscale, and ClamAV."
echo "WARNING: Ensures 4GB swap exists for ClamAV."
read -p "Continue? (y/n) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    exit 0
fi

echo "Updating system..."
apt-get update
apt-get upgrade -y
apt-get install -y fail2ban unattended-upgrades curl ufw

echo 'Unattended-Upgrade::Allowed-Origins {
    "${distro_id}:${distro_codename}-security";
    "${distro_id}ESMApps:${distro_codename}-apps-security";
    "${distro_id}ESM:${distro_codename}-infra-security";
};' > /etc/apt/apt.conf.d/50unattended-upgrades
echo 'APT::Periodic::Update-Package-Lists "1";' > /etc/apt/apt.conf.d/20auto-upgrades
echo 'APT::Periodic::Unattended-Upgrade "1";' >> /etc/apt/apt.conf.d/20auto-upgrades

echo "Hardening SSH configuration..."
cp /etc/ssh/sshd_config /etc/ssh/sshd_config.bak
sed -i 's/^#\?PermitRootLogin.*/PermitRootLogin no/' /etc/ssh/sshd_config
sed -i 's/^#\?PermitEmptyPasswords.*/PermitEmptyPasswords no/' /etc/ssh/sshd_config
sed -i 's/^#\?MaxAuthTries.*/MaxAuthTries 3/' /etc/ssh/sshd_config
systemctl restart sshd
systemctl enable fail2ban
systemctl start fail2ban

echo "Disabling laptop lid suspend..."
sed -i 's/^#\?HandleLidSwitch=.*/HandleLidSwitch=ignore/' /etc/systemd/logind.conf
sed -i 's/^#\?HandleLidSwitchDocked=.*/HandleLidSwitchDocked=ignore/' /etc/systemd/logind.conf
systemctl restart systemd-logind

if [ ! -f /swapfile ]; then
    echo "Creating 4GB Swap file..."
    fallocate -l 4G /swapfile
    chmod 600 /swapfile
    mkswap /swapfile
    swapon /swapfile
    echo '/swapfile none swap sw 0 0' | tee -a /etc/fstab
else
    echo "Swap file already exists."
fi

if ! command -v docker &> /dev/null; then
    echo "Installing Docker..."
    curl -fsSL https://get.docker.com | sh
    usermod -aG docker "$REAL_USER"
else
    echo "Docker already installed"
fi

if grep -q "#DNSStubListener=yes" /etc/systemd/resolved.conf || grep -q "DNSStubListener=yes" /etc/systemd/resolved.conf; then
    echo "Freeing port 53 for Pi-hole..."
    sed -r -i.orig 's/#?DNSStubListener=yes/DNSStubListener=no/g' /etc/systemd/resolved.conf
    rm -f /etc/resolv.conf
    echo "nameserver 1.1.1.1" > /etc/resolv.conf
    systemctl restart systemd-resolved
fi

if ! command -v tailscale &> /dev/null; then
    echo "Installing Tailscale..."
    curl -fsSL https://tailscale.com/install.sh | sh
else
    echo "Tailscale already installed"
fi

if ! tailscale status &> /dev/null; then
    echo "Please authenticate Tailscale:"
    echo "  sudo tailscale up --ssh"
    read -p "Press ENTER after authenticating..."
fi

read -p "Timezone (default: UTC): " TZ
TZ=${TZ:-UTC}
read -p "LAN subnet (e.g., 192.168.1.0/24): " LAN_SUBNET
LAN_SUBNET=${LAN_SUBNET:-192.168.1.0/24}
read -p "Media path (default: /mnt/external/media): " MEDIA_PATH
MEDIA_PATH=${MEDIA_PATH:-/mnt/external/media}

echo ""
read -p "Do you have existing credentials from a previous deployment? (y/n): " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    echo "Please provide your existing credentials:"
    read -p "Tailscale domain (e.g., https://hostname.ts.net or hostname.ts.net): " TAILSCALE_INPUT
    TAILSCALE_DOMAIN=$(echo "$TAILSCALE_INPUT" | sed -E 's|https?://||' | sed -E 's|/.*||')
    read -p "Nextcloud admin username: " NC_USER
    read -s -p "Nextcloud admin password: " NC_PASS
    echo
    read -s -p "Pi-hole web password: " PH_PASS
    echo
    read -p "MollySocket VAPID private key: " VAPID_KEY
    echo ""
else
    echo "Generating new credentials..."
    TS_HOSTNAME=$(tailscale status --json 2>/dev/null | grep -oP '"HostName":"\K[^"]+' || echo "")
    TS_SUFFIX=$(tailscale status --json 2>/dev/null | grep -oP '"MagicDNSSuffix":"\K[^"]+' || echo "ts.net")
    [ -z "$TS_HOSTNAME" ] && read -p "Enter Tailscale hostname: " TS_HOSTNAME
    TAILSCALE_DOMAIN="${TS_HOSTNAME}.${TS_SUFFIX}"
    NC_USER="admin"
    NC_PASS=$(openssl rand -base64 24 | tr -d "=+/" | head -c 20)
    PH_PASS=$(openssl rand -base64 24 | tr -d "=+/" | head -c 20)
    echo "Generating MollySocket VAPID key..."
    VAPID_KEY=$(docker run --rm ghcr.io/mollyim/mollysocket:latest vapid gen 2>/dev/null | tail -1)
fi

cat > .env << EOF
TZ=$TZ
TAILSCALE_DOMAIN=$TAILSCALE_DOMAIN
NEXTCLOUD_ADMIN_USER=$NC_USER
NEXTCLOUD_ADMIN_PASSWORD=$NC_PASS
NEXTCLOUD_TRUSTED_DOMAINS=localhost $LAN_SUBNET 100.64.0.0/10 $TAILSCALE_DOMAIN
PIHOLE_WEBPASSWORD=$PH_PASS
MOLLY_VAPID_PRIVKEY=$VAPID_KEY
MEDIA_PATH=$MEDIA_PATH
NEXTCLOUD_IMAGE=nextcloud:latest
PIHOLE_IMAGE=pihole/pihole:latest
JELLYFIN_IMAGE=jellyfin/jellyfin:latest
MOLLYSOCKET_IMAGE=ghcr.io/mollyim/mollysocket:latest
CLAMAV_IMAGE=clamav/clamav:latest
EOF

chmod 600 .env
chown "$REAL_USER:$REAL_USER" .env

echo "Configuring firewall..."
ufw default deny incoming
ufw default allow outgoing
ufw allow from "$LAN_SUBNET" to any port 22 proto tcp comment 'SSH-LAN'
ufw allow from "$LAN_SUBNET" to any port 53 comment 'DNS-LAN'
ufw allow from "$LAN_SUBNET" to any port 8081 proto tcp comment 'PiHole-Web'
ufw allow from "$LAN_SUBNET" to any port 8080 proto tcp comment 'Nextcloud'
ufw allow from "$LAN_SUBNET" to any port 8096 proto tcp comment 'Jellyfin'
ufw allow in on tailscale0 comment 'Allow-Tailscale-Traffic'
echo "y" | ufw enable

echo "Configuring Tailscale Serve..."
tailscale serve reset
tailscale serve --bg --set-path / http://localhost:8091
tailscale serve --bg --set-path /ntfy http://localhost:8082
tailscale serve --bg --https 8097 --set-path / http://localhost:8096

docker compose up -d

SERVER_IP=$(hostname -I | awk '{print $1}')
echo ""
echo "Setup complete!"
echo "Local Access:"
echo "  Nextcloud:   http://${SERVER_IP}:8080"
echo "  Pi-hole:     http://${SERVER_IP}:8081/admin"
echo "  Jellyfin:    http://${SERVER_IP}:8096"
echo ""
echo "Tailscale HTTPS Access:"
echo "  MollySocket: https://${TAILSCALE_DOMAIN}"
echo "  ntfy UI:     https://${TAILSCALE_DOMAIN}/ntfy"
echo "  Jellyfin:    https://${TAILSCALE_DOMAIN}:8097"
echo ""
echo "Credentials saved in .env"
