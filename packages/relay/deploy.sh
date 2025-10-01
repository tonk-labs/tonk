#!/bin/bash
set -e

echo "🚀 Deploying Tonk Relay..."

cd /home/ec2-user/tonk

echo "📥 Pulling latest changes..."
git fetch origin
git checkout shared/feat/latergram
git pull origin shared/feat/latergram

echo "📦 Installing dependencies..."
pnpm install

echo "🔨 Rebuilding core-js (includes WASM)..."
cd packages/core-js
pnpm build

echo "🔄 Restarting relay service..."
sudo systemctl restart relay.service

echo "✅ Deployment complete!"
echo ""
echo "Service status:"
sudo systemctl status relay.service --no-pager
echo ""
echo "To view logs: sudo journalctl -u relay.service -f"
