#!/usr/bin/env bash
# Capture the source pane, render matches in a replica, then swap back.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="${TMUX_FLASH_BINARY:-$(tmux show-option -gqv @_tmux-flash-binary 2>/dev/null || true)}"
if [[ -z "$BIN" || ! -x "$BIN" ]]; then
  BIN="$("$DIR/scripts/install-tmux-flash")"
  tmux set-option -gq @_tmux-flash-binary "$BIN"
fi

snapshot="$(tmux display-message -p '#{pane_id};#{scroll_position};#{copy_cursor_x};#{copy_cursor_y};#{pane_width};#{pane_height};#{pane_in_mode}' \; show-options -g)"
info="${snapshot%%$'\n'*}"
user_options="${snapshot#*$'\n'}"
IFS=';' read -r pane scroll cx cy width height inmode <<<"$info"
[[ "$inmode" == 1 ]] || exit 0

start=$((0 - scroll))
end=$((height - scroll - 1))
capture="$(mktemp)"
trap 'rm -f -- "$capture"' EXIT HUP INT TERM
tmux capture-pane -p -t "$pane" -S "$start" -E "$end" > "$capture"

labels=""
theme=""
while IFS=' ' read -r name value; do
  case "$name" in
    @flash-labels) labels="$value" ;;
    @flash-colors) theme="$value" ;;
  esac
done <<<"$user_options"

args=(--orig "$pane" --capture "$capture" --cx "$cx" --cy "$cy" --width "$width" --height "$height" --labels "${labels:-asdfghjklqwertyuiopzxcvbnm}")
if [[ -n "$theme" ]]; then
  IFS=',' read -r -a pairs <<<"$theme"
  for pair in "${pairs[@]}"; do
    name="${pair%%=*}"
    value="${pair#*=}"
    case "$name" in
      backdrop|match-fg|match-bg|current-fg|current-bg|label-fg|label-bg|status|status-error)
        if [[ -n "$value" && "$pair" != "$name" ]]; then args+=("--color-$name" "$value"); fi
        ;;
    esac
  done
fi

command="'$BIN'"
for arg in "${args[@]}"; do
  printf -v quoted '%q' "$arg"
  command+=" $quoted"
done
replica="$(tmux new-window -dP -F '#{pane_id}' -n flash "exec $command")"
# Match the original pane geometry before swap so tmux cannot reflow and reset
# the copy-mode viewport in split panes.
tmux resize-window -t "$replica" -x "$width" -y "$height"
tmux swap-pane -s "$replica" -t "$pane" \; select-pane -t "$replica"
