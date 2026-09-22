#!/usr/bin/env bash
# Cloud Agent 环境安装脚本：幂等地准备 noxcode（Tauri 2 + React 19）的开发依赖。
set -euo pipefail

echo "[install] 安装 Tauri/Linux 系统依赖 (webkit2gtk, gtk3, dbus, x11 等)"
export DEBIAN_FRONTEND=noninteractive
sudo apt-get update -y
sudo apt-get install -y --no-install-recommends \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libdbus-1-dev \
  libssl-dev \
  libx11-dev \
  libxdo-dev \
  build-essential \
  pkg-config \
  curl \
  wget \
  file

echo "[install] 确保 Rust 工具链支持 edition2024 (需 >= 1.85，仓库按 1.94 验证)"
rustup toolchain install stable --profile minimal --component clippy
rustup default stable
rustc --version
cargo --version

echo "[install] 安装前端依赖 (npm ci)"
npm ci

echo "[install] 完成"
