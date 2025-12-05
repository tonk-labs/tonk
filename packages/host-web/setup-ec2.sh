#!/bin/bash
set -e

echo "🚀 Setting up Tonk Host-Web on EC2..."

if [ "$(whoami)" != "ec2-user" ]; then
  echo "⚠️  This script should be run as ec2-user"
  exit 1
fi

cd /home/ec2-user

echo "📦 Installing system dependencies..."
sudo yum update -y
sudo yum install -y git gcc make

echo "🦀 Installing Rust..."
if ! command -v rustc &>/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
else
  echo "✓ Rust already installed"
fi

echo "📦 Installing wasm-pack..."
if ! command -v wasm-pack &>/dev/null; then
  curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
else
  echo "✓ wasm-pack already installed"
fi

echo "📦 Installing Node.js..."
if ! command -v node &>/dev/null; then
  curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash
  export NVM_DIR="$HOME/.nvm"
  [ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"
  nvm install 22
  nvm use 22
else
  echo "✓ Node.js already installed"
fi

echo "📦 Installing Bun..."
if ! command -v bun &>/dev/null; then
  curl -fsSL https://bun.sh/install | bash
  export BUN_INSTALL="$HOME/.bun"
  export PATH="$BUN_INSTALL/bin:$PATH"
else
  echo "✓ Bun already installed"
fi

echo "📥 Cloning repository..."
if [ ! -d "/home/ec2-user/tonk" ]; then
  git clone https://github.com/tonk-labs/tonk.git tonk
else
  echo "✓ Repository already cloned"
fi

cd /home/ec2-user/tonk

echo "📦 Installing dependencies..."
bun install

echo "🔨 Building core-js (includes WASM)..."
cd packages/core-js
bun run build

echo "🔨 Building host-web..."
cd ../host-web
bun run build

echo "⚙️  Setting up systemd service..."
sudo cp /home/ec2-user/tonk/packages/host-web/host-web.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable host-web.service
sudo systemctl start host-web.service

echo "✅ Setup complete!"
echo ""
echo "Service status:"
sudo systemctl status host-web.service --no-pager
echo ""
echo "To view logs: sudo journalctl -u host-web.service -f"
echo "To deploy updates: cd /home/ec2-user/tonk/packages/host-web && ./deploy.sh"
