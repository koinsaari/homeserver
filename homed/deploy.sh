#!/bin/bash
set -euo pipefail

REPO="koinsaari/homeserver"
BINARY_URL="https://github.com/$REPO/releases/latest/download/homed"
INSTALL_DIR="/opt/homed"

echo "Downloading latest homed binary..."
curl -fL -o /tmp/homed "$BINARY_URL"

echo "Stopping homed service..."
sudo systemctl stop homed

echo "Installing binary..."
sudo install -m 755 /tmp/homed "$INSTALL_DIR/homed"
rm /tmp/homed

echo "Starting homed service..."
sudo systemctl start homed

echo "Done. Status:"
sudo systemctl status homed --no-pager
