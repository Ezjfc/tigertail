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
"${SSH[@]}" 'mv /home/root/tigertail/tigertaild.new /home/root/tigertail/tigertaild && systemctl daemon-reload'

echo "Deployed. Enable with: ${SSH[*]} systemctl enable --now tigertaild"
