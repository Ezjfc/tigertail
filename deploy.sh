#!/usr/bin/env bash
# Build and deploy tigertaild to the tablet. Run from the flake devShell.
#
# Usage: [SSH_KEY=~/.ssh/id_ed25519.remarkable] ./deploy.sh [root@10.11.99.1]
set -eu

HOST="${1:-root@10.11.99.1}"
SSH_KEY="${SSH_KEY:-$HOME/.ssh/id_ed25519.remarkable}"
SSH=(ssh -i "$SSH_KEY" -o BatchMode=yes "$HOST")
BIN=target/armv7-unknown-linux-musleabihf/release/tigertaild

cargo build -p tigertaild --release

# /home/root survives reMarkable OS updates; the systemd unit does not and is
# re-installed by re-running this script after an update.
"${SSH[@]}" 'mkdir -p /home/root/tigertail'
scp -i "$SSH_KEY" -O "$BIN" "$HOST":/home/root/tigertail/tigertaild.new
scp -i "$SSH_KEY" -O data/tigertaild.service "$HOST":/etc/systemd/system/tigertaild.service
# Seed the config only if none exists: it is user/GUI-owned state.
scp -i "$SSH_KEY" -O tigertail.toml.example "$HOST":/home/root/tigertail/tigertail.toml.example
"${SSH[@]}" '[ -e /home/root/tigertail/tigertail.toml ] || cp /home/root/tigertail/tigertail.toml.example /home/root/tigertail/tigertail.toml'
"${SSH[@]}" 'mv /home/root/tigertail/tigertaild.new /home/root/tigertail/tigertaild && systemctl daemon-reload && systemctl try-restart tigertaild'

echo "Deployed (running service restarted). Enable at boot with: ${SSH[*]} systemctl enable tigertaild"
