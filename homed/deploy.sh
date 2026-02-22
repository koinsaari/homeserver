#!/bin/bash
set -euo pipefail

REPO="koinsaari/homeserver"
BINARY_URL="https://github.com/$REPO/releases/latest/download/homed"
INSTALL_DIR="/opt/homed"
SERVICE_NAME="homed.service"

sudo mkdir -p "$INSTALL_DIR"

echo "Downloading latest homed binary..."
curl -fL -o /tmp/homed "$BINARY_URL"

echo "Checking if service is loaded..."
if systemctl list-unit-files "$SERVICE_NAME" >/dev/null 2>&1; then
    echo "Stopping existing homed service..."
    sudo systemctl stop homed
else
    echo "Service not found, skipping stopping."
fi

echo "Installing binary..."
sudo install -m 755 /tmp/homed "$INSTALL_DIR/homed"
rm /tmp/homed

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
echo "Installing service file..."
sudo cp "$SCRIPT_DIR/homed.service" /etc/systemd/system/homed.service

if [ ! -f "$INSTALL_DIR/config.toml" ]; then
    sudo cp "$SCRIPT_DIR/config.example.toml" "$INSTALL_DIR/config.toml"
    echo "Copied config.example.toml to $INSTALL_DIR/config.toml"
    echo "IMPORTANT: Edit $INSTALL_DIR/config.toml with your actual values before starting"
fi

echo "Reloading systemd and starting service..."
sudo systemctl daemon-reload
sudo systemctl enable homed
sudo systemctl start homed

echo "Done. Status:"
sudo systemctl status homed --no-pager
