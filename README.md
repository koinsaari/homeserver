# Simple Home Server

My self-hosted setup running on an old laptop.

## What's Running

- **Nextcloud** for Cloud storage + photo backup
- **Pi-hole** for Network-wide ad blocking
- **Jellyfin** for Media server
- **MollySocket** for Signal notifications
- **ClamAV** for Virus scanning

## Setup

```bash
git clone https://github.com/koinsaari/homeserver.git
cd homeserver
sudo ./setup.sh
```

Script handles everything: Docker, Tailscale, firewall, passwords.

## Security

No WAN exposure. Remote access only via Tailscale. UFW firewall, full-disk encryption, automatic security updates.

## Hardware

2015 laptop, 8GB RAM, LUKS-encrypted SSD.

## License

MIT
