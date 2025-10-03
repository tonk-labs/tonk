#!/bin/bash
set -e

echo "🚀 Setting up Tonk Relay on EC2..."

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

echo "📦 Installing pnpm..."
if ! command -v pnpm &>/dev/null; then
  curl -fsSL https://get.pnpm.io/install.sh | sh -
  export PNPM_HOME="/home/ec2-user/.local/share/pnpm"
  export PATH="$PNPM_HOME:$PATH"
else
  echo "✓ pnpm already installed"
fi

echo "📥 Cloning repository..."
if [ ! -d "/home/ec2-user/tonk" ]; then
  git clone https://github.com/tonk-labs/tonk.git tonk
else
  echo "✓ Repository already cloned"
fi

cd /home/ec2-user/tonk

echo "📦 Installing dependencies..."
pnpm install
cd packages/relay
pnpm install

echo "🔨 Building core-js (includes WASM)..."
cd ../core-js
pnpm install
pnpm build

echo "⚙️  Setting up systemd service..."
sudo cp /home/ec2-user/tonk/packages/relay/relay.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable relay.service
sudo systemctl start relay.service

echo "✅ Setup complete!"
echo ""
echo "Service status:"
sudo systemctl status relay.service --no-pager
echo ""
echo "To view logs: sudo journalctl -u relay.service -f"
echo "To deploy updates: cd /home/ec2-user/tonk/packages/relay && ./deploy.sh"
