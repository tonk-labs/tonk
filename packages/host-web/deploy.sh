#!/bin/bash
set -e

echo "🚀 Deploying Tonk Host-Web..."

cd /home/ec2-user/tonk

echo "📥 Pulling latest changes..."
git fetch origin
git checkout main
git pull origin main

echo "📦 Installing dependencies..."
pnpm install

echo "🔨 Rebuilding core-js (includes WASM)..."
cd packages/core-js
pnpm build

echo "🔨 Rebuilding host-web..."
cd ../host-web
pnpm build

echo "🔄 Restarting host-web service..."
sudo systemctl restart host-web.service

echo "✅ Deployment complete!"
echo ""
echo "Service status:"
sudo systemctl status host-web.service --no-pager
echo ""
echo "To view logs: sudo journalctl -u host-web.service -f"
