#!/bin/bash
set -euo pipefail

CONFIG_FILE="${BACKUP_CONFIG:-/etc/backup/backup.env}"

log() {
    echo "$(date '+%Y-%m-%d %H:%M:%S') $1"
}

send_alert() {
    local priority="$1"
    local message="$2"

    if [ -n "${NTFY_URL:-}" ] && [ -n "${NTFY_TOPIC:-}" ]; then
        local auth_args=()
        if [ -n "${NTFY_TOKEN:-}" ]; then
            auth_args=(-H "Authorization: Bearer $NTFY_TOKEN")
        fi
        curl -s \
            -H "Priority: $priority" \
            "${auth_args[@]}" \
            -d "$message" \
            "${NTFY_URL}/${NTFY_TOPIC}" > /dev/null 2>&1 || true
    fi
}

cleanup() {
    if mountpoint -q "$BACKUP_MOUNT" 2>/dev/null; then
        log "Unmounting $BACKUP_MOUNT"
        umount "$BACKUP_MOUNT" || true
    fi
    if [ -e /dev/mapper/backup-vault ]; then
        log "Closing LUKS"
        cryptsetup close backup-vault || true
    fi
}

trap cleanup EXIT INT TERM

if [ ! -f "$CONFIG_FILE" ]; then
    echo "Error: Config file not found: $CONFIG_FILE"
    echo "Copy backup.env.example to $CONFIG_FILE and configure it."
    exit 1
fi

source "$CONFIG_FILE"

: "${BACKUP_DRIVE_UUID:?BACKUP_DRIVE_UUID not set in $CONFIG_FILE}"
: "${BACKUP_MOUNT:=/mnt/backup}"
: "${BACKUP_KEYFILE:=/etc/backup/backup.key}"
: "${BACKUP_SOURCES:?BACKUP_SOURCES not set in $CONFIG_FILE}"
: "${ALERT_ON_MISSING:=false}"

DRIVE_PATH="/dev/disk/by-uuid/$BACKUP_DRIVE_UUID"

if [ ! -e "$DRIVE_PATH" ]; then
    log "Drive not connected (UUID: $BACKUP_DRIVE_UUID)"
    if [ "$ALERT_ON_MISSING" = "true" ]; then
        send_alert "low" "Backup skipped: drive not connected"
    fi
    exit 0
fi

if [ ! -f "$BACKUP_KEYFILE" ]; then
    log "Error: Keyfile not found: $BACKUP_KEYFILE"
    send_alert "urgent" "Backup FAILED: keyfile not found"
    exit 1
fi

START_TIME=$(date +%s)
log "Starting backup"

log "Opening LUKS partition"
if ! cryptsetup open "$DRIVE_PATH" backup-vault --key-file "$BACKUP_KEYFILE"; then
    log "Error: Failed to unlock drive"
    send_alert "urgent" "Backup FAILED: could not unlock drive"
    exit 1
fi

log "Mounting filesystem"
mkdir -p "$BACKUP_MOUNT"
if ! mount /dev/mapper/backup-vault "$BACKUP_MOUNT"; then
    log "Error: Failed to mount drive"
    send_alert "urgent" "Backup FAILED: could not mount drive"
    exit 1
fi

TOTAL_FILES=0
FAILED=0

for SOURCE in $BACKUP_SOURCES; do
    if [ ! -d "$SOURCE" ]; then
        log "Warning: Source not found, skipping: $SOURCE"
        continue
    fi

    DIRNAME=$(basename "$SOURCE")
    DEST="$BACKUP_MOUNT/$DIRNAME"

    log "Copying $SOURCE -> $DEST"

    if OUTPUT=$(rclone copy "$SOURCE" "$DEST" --checksum --stats-one-line 2>&1); then
        FILES=$(echo "$OUTPUT" | grep -oP 'Transferred:\s+\K\d+' || echo "0")
        FILES=${FILES:-0}
        TOTAL_FILES=$((TOTAL_FILES + FILES))
        log "Copied $FILES files from $DIRNAME"
    else
        log "Error: rclone failed for $SOURCE"
        FAILED=1
    fi
done

DURATION=$(( $(date +%s) - START_TIME ))
MINUTES=$((DURATION / 60))
SECONDS=$((DURATION % 60))

if [ "$FAILED" -eq 1 ]; then
    log "Backup completed with errors in ${MINUTES}m ${SECONDS}s"
    send_alert "high" "Backup completed with errors: ${TOTAL_FILES} files in ${MINUTES}m"
    exit 1
else
    log "Backup completed: ${TOTAL_FILES} files in ${MINUTES}m ${SECONDS}s"
    send_alert "default" "Backup completed: ${TOTAL_FILES} files in ${MINUTES}m"
fi
