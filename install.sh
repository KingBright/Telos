#!/bin/bash

set -e

echo "Building Telos project..."
cargo build --release

# 动态获取 cargo 的 target 目录
TARGET_DIR=$(cargo metadata --format-version 1 --no-deps 2>/dev/null | \
    grep -o '"target_directory":"[^"]*"' | \
    sed 's/"target_directory":"//;s/"//')

if [ -z "$TARGET_DIR" ]; then
    echo "Error: Failed to determine target directory"
    exit 1
fi

echo "Installing binaries from $TARGET_DIR/release to ~/.cargo/bin..."
mkdir -p ~/.cargo/bin
cp "$TARGET_DIR/release/telos_cli" "$TARGET_DIR/release/telos_daemon" ~/.cargo/bin/
mv ~/.cargo/bin/telos_cli ~/.cargo/bin/telos

echo "Installation successful."

CONFIG_FILE="$HOME/.telos_config.toml"
if [ -f "$CONFIG_FILE" ]; then
    echo "Configuration found. Starting telos_daemon..."
    # Check if daemon is already running
    if pgrep -x "telos_daemon" > /dev/null
    then
        echo "telos_daemon is already running."
    else
        ~/.cargo/bin/telos_daemon > /dev/null 2>&1 &
        echo "telos_daemon started in the background."
    fi
else
    echo "No configuration found."
    echo "Please run 'telos init' to set up your environment."
fi
