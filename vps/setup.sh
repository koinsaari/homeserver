#!/bin/bash
set -e

CURRENT_USER=$(whoami)

if [ "$CURRENT_USER" = "root" ]; then
    echo "Running as root. Checking for non-root user setup..."
    read -p "Create new non-root user? (y/n, default: y): " CREATE_USER
    CREATE_USER=${CREATE_USER:-y}

    if [ "$CREATE_USER" = "y" ]; then
        read -p "Enter username for the new non-root user: " NEW_USER
        if [ -z "$NEW_USER" ]; then
            echo "Error: Username cannot be empty"
            exit 1
        fi

        if id "$NEW_USER" &>/dev/null; then
            echo "User $NEW_USER already exists, using existing user"
        else
            echo "Creating user $NEW_USER..."
            useradd -m -s /bin/bash $NEW_USER
            usermod -aG sudo $NEW_USER

            mkdir -p /home/$NEW_USER/.ssh
            if [ -f /root/.ssh/authorized_keys ]; then
                cp /root/.ssh/authorized_keys /home/$NEW_USER/.ssh/
                chown -R $NEW_USER:$NEW_USER /home/$NEW_USER/.ssh
                chmod 700 /home/$NEW_USER/.ssh
                chmod 600 /home/$NEW_USER/.ssh/authorized_keys
            fi
        fi
    else
        NEW_USER="root"
    fi
else
    echo "Running as non-root user: $CURRENT_USER"
    NEW_USER="$CURRENT_USER"
fi

echo "Using user: $NEW_USER"

if [ "$CURRENT_USER" = "root" ]; then
    apt update && apt upgrade -y
    apt install -y curl git ufw ca-certificates gnupg sudo
else
    echo "Skipping apt operations (run as root for system updates)"
fi

if [ -x "$(command -v ufw)" ] && [ "$CURRENT_USER" = "root" ]; then
    echo "Configuring firewall..."
    ufw default deny incoming
    ufw default allow outgoing
    ufw allow 22/tcp
    ufw allow 80/tcp
    ufw allow 443/tcp
    ufw allow 3478/udp
    ufw allow 3478/tcp
    ufw --force enable
else
    echo "Skipping firewall setup (ufw not available or not running as root)"
fi

if ! [ -x "$(command -v docker)" ]; then
    echo "Installing Docker..."
    curl -fsSL https://get.docker.com -o get-docker.sh
    sh get-docker.sh
    rm get-docker.sh
else
    echo "Docker already installed"
fi

if groups $NEW_USER | grep -q '\bdocker\b'; then
    echo "User $NEW_USER already in docker group"
else
    echo "Adding $NEW_USER to docker group..."
    if [ "$CURRENT_USER" = "root" ]; then
        usermod -aG docker $NEW_USER
    else
        sudo usermod -aG docker $NEW_USER
    fi
fi

if [ "$CURRENT_USER" = "root" ] && [ "$NEW_USER" != "root" ]; then
    if grep -q "^PermitRootLogin no" /etc/ssh/sshd_config; then
        echo "Root login already disabled"
    else
        echo "Disabling root SSH login..."
        sed -i 's/^#*PermitRootLogin.*/PermitRootLogin no/' /etc/ssh/sshd_config
        systemctl restart ssh
        echo "Root SSH login disabled."
    fi
fi

echo "Initializing stack directories and network..."
docker network create proxy-net || true

mkdir -p ntfy_data ntfy_cache mollysocket_data caddy_data caddy_config
chown -R 1000:1000 ntfy_data ntfy_cache caddy_data caddy_config
chmod -R 700 mollysocket_data

echo ""
echo "Setup complete!"
echo "User: $NEW_USER"
if [ "$CURRENT_USER" = "root" ] && [ "$NEW_USER" != "root" ]; then
    echo ""
    echo "IMPORTANT: Test SSH access with '$NEW_USER' in a separate terminal before closing this session!"
    echo "If docker group was just added, $NEW_USER needs to log out and back in for changes to take effect."
fi
