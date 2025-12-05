#!/bin/bash
set -e

EC2_HOST="ec2-13-48-6-167.eu-north-1.compute.amazonaws.com"
EC2_USER="ec2-user"

echo "🚀 Deploying Tonk Host-Web to ${EC2_HOST}..."

ssh "${EC2_USER}@${EC2_HOST}" <<'ENDSSH'
set -e

echo "📥 Pulling latest changes..."
cd /home/ec2-user/tonk
git fetch origin
git checkout main
git pull origin main

echo "📦 Installing dependencies..."
bun install

echo "🔨 Rebuilding core-js (includes WASM)..."
cd packages/core-js
bun run build

echo "🔨 Rebuilding host-web..."
cd ../host-web
bun run build

echo "🔄 Restarting host-web service..."
sudo systemctl restart host-web.service

echo "✅ Deployment complete!"
echo ""
echo "Service status:"
sudo systemctl status host-web.service --no-pager
echo ""
echo "To view logs: sudo journalctl -u host-web.service -f"
ENDSSH
