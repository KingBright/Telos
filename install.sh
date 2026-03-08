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

OS="$(uname -s)"

# Stop any running daemon FIRST (before replacing binary)
if [ "$OS" = "Darwin" ]; then
    PLIST_PATH="$HOME/Library/LaunchAgents/com.telos.daemon.plist"
    launchctl unload "$PLIST_PATH" 2>/dev/null || true
fi
pkill -9 telos_daemon 2>/dev/null || true
sleep 1

echo "Installing binaries from $TARGET_DIR/release to ~/.cargo/bin..."
mkdir -p ~/.cargo/bin

# Remove old binaries first to avoid macOS inode/vnode conflicts with zombie processes
rm -f ~/.cargo/bin/telos_daemon ~/.cargo/bin/telos 2>/dev/null || true

cp "$TARGET_DIR/release/telos_cli" "$TARGET_DIR/release/telos_daemon" ~/.cargo/bin/
mv ~/.cargo/bin/telos_cli ~/.cargo/bin/telos

echo "Installation successful."

CONFIG_FILE="$HOME/.telos_config.toml"
LOG_DIR="$HOME/.telos_logs"
mkdir -p "$LOG_DIR"

# Configure auto-start files
if [ "$OS" = "Darwin" ]; then
    PLIST_PATH="$HOME/Library/LaunchAgents/com.telos.daemon.plist"
    mkdir -p "$HOME/Library/LaunchAgents"

    cat <<EOF > "$PLIST_PATH"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.telos.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>$HOME/.cargo/bin/telos_daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>$LOG_DIR/daemon.log</string>
    <key>StandardErrorPath</key>
    <string>$LOG_DIR/daemon.err</string>
</dict>
</plist>
EOF
elif [ "$OS" = "Linux" ]; then
    SERVICE_PATH="$HOME/.config/systemd/user/telos-daemon.service"
    mkdir -p "$HOME/.config/systemd/user"

    cat <<EOF > "$SERVICE_PATH"
[Unit]
Description=Telos Daemon
After=network.target

[Service]
ExecStart=$HOME/.cargo/bin/telos_daemon
Restart=always

[Install]
WantedBy=default.target
EOF
fi

if [ -f "$CONFIG_FILE" ]; then
    echo "Configuration found. Starting telos_daemon..."
    if [ "$OS" = "Darwin" ]; then
        launchctl load -w "$PLIST_PATH"
        # Wait for daemon to start and verify
        echo -n "Waiting for daemon to start"
        for i in $(seq 1 10); do
            if lsof -i :3000 -P 2>/dev/null | grep -q LISTEN; then
                echo ""
                echo "telos_daemon started successfully and listening on port 3000."
                echo "Daemon logs: $LOG_DIR/daemon.log"
                exit 0
            fi
            echo -n "."
            sleep 1
        done
        echo ""
        echo "Warning: Daemon may not have started via launchd. Starting directly..."
        nohup ~/.cargo/bin/telos_daemon >> "$LOG_DIR/daemon.log" 2>> "$LOG_DIR/daemon.err" &
        sleep 2
        if lsof -i :3000 -P 2>/dev/null | grep -q LISTEN; then
            echo "telos_daemon started successfully (direct) on port 3000."
        else
            echo "Error: telos_daemon failed to start. Check logs: $LOG_DIR/daemon.err"
        fi
    elif [ "$OS" = "Linux" ] && command -v systemctl >/dev/null 2>&1; then
        systemctl --user daemon-reload
        systemctl --user enable telos-daemon.service
        systemctl --user restart telos-daemon.service
        echo "telos_daemon started and configured to run on boot via systemd."
    else
        if pgrep -x "telos_daemon" > /dev/null; then
            echo "telos_daemon is already running."
        else
            nohup ~/.cargo/bin/telos_daemon >> "$LOG_DIR/daemon.log" 2>> "$LOG_DIR/daemon.err" &
            sleep 2
            echo "telos_daemon started in the background."
        fi
    fi
else
    echo "No configuration found."
    echo "Please run 'telos init' to set up your environment."
fi

