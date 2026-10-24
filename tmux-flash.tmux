#!/usr/bin/env bash
# TPM entry point for tmux-flash.
set -euo pipefail

CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
key="$(tmux show-option -gqv '@flash-key')"

tmux bind-key -T copy-mode-vi "${key:-s}" run-shell -b "$CURRENT_DIR/flash.sh"

# Resolve/download the matching prebuilt binary on plugin load. No Rust toolchain
# is needed on the user's machine; flash.sh retries if this first install fails.
if binary="$("$CURRENT_DIR/scripts/install-tmux-flash" 2>/dev/null)"; then
  tmux set-option -gq @_tmux-flash-binary "$binary"
else
  tmux set-option -gu @_tmux-flash-binary 2>/dev/null || true
  tmux display-message -d 5000 "tmux-flash: prebuilt binary unavailable; check network and GitHub authentication"
fi
