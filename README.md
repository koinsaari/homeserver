# Simple Home Server

My self-hosted setup running on an old laptop.

## What's Running

* **Nextcloud**: Cloud storage and photo backup with automated syncing.
* **Pi-hole**: Network-wide ad and tracker blocker that functions as a DNS sinkhole.
* **Jellyfin**: Private media server for streaming any type of media.
* **MollySocket**: UnifiedPush provider that monitors Signal for new messages.
* **ntfy**: Private notification server that delivers MollySocket alerts to phone via Tailscale.
* **ClamAV**: Antivirus engine integrated with Nextcloud for automated file scanning.

## Setup

```bash
git clone https://github.com/koinsaari/homeserver.git
cd homeserver
chmod +x setup.sh
sudo ./setup.sh
```

Script handles everything: Docker, Tailscale, firewall, passwords.

## Security

No WAN exposure. Remote access only via Tailscale. UFW firewall, full-disk encryption, automatic security updates.

## Hardware

Some HP 2015 laptop, 8GB RAM, LUKS-encrypted SSD.

## License

MIT
