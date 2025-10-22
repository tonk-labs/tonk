#!/bin/bash
set -e

EC2_HOST="ec2-16-16-146-55.eu-north-1.compute.amazonaws.com"
EC2_USER="ec2-user"

echo "🚀 Deploying Tonk Relay to ${EC2_HOST}..."

ssh "${EC2_USER}@${EC2_HOST}" <<'ENDSSH'
set -e

echo "📥 Pulling latest changes..."
cd /home/ec2-user/tonk
git fetch origin
git checkout main
git pull origin main

echo "📦 Installing dependencies..."
pnpm install

echo "🔨 Rebuilding relay..."
cd packages/relay
cargo build --release

echo "🔨 Rebuilding core-js (includes WASM)..."
cd ../core-js
pnpm build --frozen-lockfile

echo "🔄 Restarting relay service..."
sudo systemctl restart relay.service

echo "✅ Deployment complete!"
echo ""
echo "Service status:"
sudo systemctl status relay.service --no-pager
echo ""
echo "To view logs: sudo journalctl -u relay.service -f"
ENDSSH
