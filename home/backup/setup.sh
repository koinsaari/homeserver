#!/bin/bash
#
# setup.sh: One-time LUKS + ext4 setup for backup drive
#
# This script prepares a USB drive for encrypted backups:
#   - LUKS2 encryption (industry standard, works on any Linux)
#   - ext4 filesystem (journaled, corruption-resistant)
#   - Keyfile for automated daily backups
#   - Passphrase for manual recovery on any machine
#
# Usage:
#   sudo ./setup.sh /dev/sdX
#
# After setup, the drive can be unlocked two ways:
#   1. Automatically by backup.sh using the keyfile
#   2. Manually on any Linux: cryptsetup open /dev/sdX backup-vault

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

error() { echo -e "${RED}Error: $1${NC}" >&2; exit 1; }
warn() { echo -e "${YELLOW}$1${NC}"; }
info() { echo -e "${GREEN}$1${NC}"; }

cleanup() {
    if [ -b "/dev/mapper/backup-setup" ]; then
        cryptsetup close backup-setup || true
    fi
}
trap cleanup EXIT

if [ "$EUID" -ne 0 ]; then
    error "Must run as root (sudo ./setup.sh /dev/sdX)"
fi

if [ $# -ne 1 ]; then
    echo "Usage: $0 /dev/sdX"
    echo ""
    echo "Available drives:"
    lsblk -d -o NAME,SIZE,MODEL,TRAN | grep -E "usb|NAME"
    exit 1
fi

DEVICE="$1"

if [ ! -b "$DEVICE" ]; then
    error "$DEVICE is not a block device"
fi

if mount | grep -q "^$DEVICE"; then
    error "$DEVICE is currently mounted. Unmount it first."
fi

if mount | grep -q "^${DEVICE}[0-9]"; then
    error "A partition on $DEVICE is currently mounted. Unmount it first."
fi

ROOT_DISK=$(lsblk -no PKNAME "$(findmnt -n -o SOURCE /)" 2>/dev/null || echo "")
if [ "/dev/$ROOT_DISK" = "$DEVICE" ]; then
    error "$DEVICE is your system disk!"
fi

echo ""
warn "╔════════════════════════════════════════════════════════════╗"
warn "║  WARNING: THIS WILL PERMANENTLY ERASE ALL DATA ON DEVICE   ║"
warn "╚════════════════════════════════════════════════════════════╝"
echo ""
echo "Device: $DEVICE"
echo ""
echo "Current contents:"
lsblk -o NAME,SIZE,FSTYPE,LABEL,MOUNTPOINT "$DEVICE" 2>/dev/null || lsblk "$DEVICE"
echo ""
echo "This operation cannot be undone."
echo ""
read -p "Type the device name again to confirm (e.g., /dev/sdb): " CONFIRM

if [ "$CONFIRM" != "$DEVICE" ]; then
    echo "Aborted - confirmation did not match"
    exit 1
fi

echo ""
info "=== Step 1/6: Creating LUKS2 encrypted partition ==="
echo ""
echo "You will now create a recovery passphrase."
echo ""
warn "IMPORTANT: Save this passphrase somewhere safe!"
warn "It's the ONLY way to access your data if the server is lost."
echo ""
wipefs -a "$DEVICE"
cryptsetup luksFormat --type luks2 --pbkdf argon2id "$DEVICE"

echo ""
info "=== Step 2/6: Opening encrypted partition ==="
cryptsetup open "$DEVICE" backup-setup

echo ""
info "=== Step 3/6: Creating ext4 filesystem ==="
echo "This provides journaling to protect against corruption."
mkfs.ext4 -m 0 -L backup /dev/mapper/backup-setup

echo ""
info "=== Step 4/6: Generating keyfile for automation ==="
echo "This allows the backup script to unlock the drive without a password."
mkdir -p /etc/backup
dd if=/dev/urandom of=/etc/backup/backup.key bs=4096 count=1 status=none
chmod 600 /etc/backup/backup.key
chown root:root /etc/backup/backup.key
echo "Keyfile created: /etc/backup/backup.key (root-only access)"

echo ""
info "=== Step 5/6: Adding keyfile to LUKS ==="
echo "Enter the passphrase you just created:"
cryptsetup luksAddKey "$DEVICE" /etc/backup/backup.key

echo ""
info "=== Step 6/6: Closing partition ==="
cryptsetup close backup-setup
trap - EXIT

DRIVE_UUID=$(blkid -s UUID -o value "$DEVICE")

echo ""
info "╔════════════════════════════════════════════════════════════╗"
info "║                    SETUP COMPLETE                          ║"
info "╚════════════════════════════════════════════════════════════╝"
echo ""
echo "Drive UUID: $DRIVE_UUID"
echo "Keyfile:    /etc/backup/backup.key"
echo ""
echo "Add this to /etc/backup/backup.env:"
echo ""
echo "    BACKUP_DRIVE_UUID=\"$DRIVE_UUID\""
echo ""
echo "To recover data on another Linux machine:"
echo ""
echo "    cryptsetup open /dev/sdX backup-vault"
echo "    mount /dev/mapper/backup-vault /mnt/backup"
echo "    # browse files in /mnt/backup"
echo ""