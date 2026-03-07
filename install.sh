#!/bin/bash

set -e

echo "Building Telos project..."
cargo build --release

echo "Installing binaries to ~/.cargo/bin..."
mkdir -p ~/.cargo/bin
cp target/release/telos target/release/telos_daemon ~/.cargo/bin/

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
