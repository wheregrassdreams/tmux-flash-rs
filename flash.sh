#!/usr/bin/env bash
# Capture the source pane, render matches in a replica, then swap back.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="${TMUX_FLASH_BINARY:-$(tmux show-option -gqv @_tmux-flash-binary 2>/dev/null || true)}"
if [[ -z "$BIN" || ! -x "$BIN" ]]; then
  BIN="$("$DIR/scripts/install-tmux-flash")"
  tmux set-option -gq @_tmux-flash-binary "$BIN"
fi

pane="$(tmux display-message -p '#{pane_id}')"
if [[ "$(tmux show-option -pqv -t "$pane" @tmux-flash-active)" == 1 ]]; then
  exit 0
fi

if [[ "$(tmux display-message -p -t "$pane" '#{pane_mode}')" != copy-mode ]]; then
  tmux copy-mode -t "$pane"
fi

snapshot="$(tmux display-message -p -t "$pane" '#{pane_id};#{scroll_position};#{copy_cursor_x};#{copy_cursor_y};#{pane_width};#{pane_height};#{pane_in_mode};#{window_zoomed_flag}')"
info="$snapshot"
IFS=';' read -r pane scroll cx cy width height inmode zoomed <<<"$info"
[[ "$inmode" == 1 ]] || exit 0

start=$((0 - scroll))
end=$((height - scroll - 1))
capture="$(mktemp)"
trap 'rm -f -- "$capture"' EXIT HUP INT TERM
# -M captures copy-mode's backing screen. Its default range is the bottom
# viewport, so apply scroll_position to read the viewport currently on screen.
tmux capture-pane -p -M -t "$pane" -S "$start" -E "$end" > "$capture"

# The cursor, viewport, and geometry above must describe the captured rows.
# They are queried separately from capture-pane, so reject a snapshot if the
# pane changed while the capture was being taken.
after_capture="$(tmux display-message -p -t "$pane" '#{pane_id};#{scroll_position};#{copy_cursor_x};#{copy_cursor_y};#{pane_width};#{pane_height};#{pane_in_mode};#{window_zoomed_flag}')"
if [[ "$after_capture" != "$snapshot" ]]; then
  tmux display-message -d 2500 -t "$pane" 'tmux-flash: pane changed during capture; retry flash'
  exit 0
fi
labels="$(tmux show-option -gqv '@flash-labels')"
args=(--orig "$pane" --capture "$capture" --cx "$cx" --cy "$cy" --scroll "$scroll" --width "$width" --height "$height")
if [[ -n "$labels" ]]; then args+=(--labels "$labels"); fi
if [[ "$zoomed" == 1 ]]; then args+=(--original-zoomed); fi

for name in backdrop match-fg match-bg current-fg current-bg label-fg label-bg status status-error; do
  value="$(tmux show-option -gqv "@flash-color-$name")"
  if [[ -n "$value" ]]; then args+=("--color-$name" "$value"); fi
done

command="'$BIN'"
for arg in "${args[@]}"; do
  printf -v quoted '%q' "$arg"
  command+=" $quoted"
done
replica="$(tmux new-window -dP -F '#{pane_id}' -n flash "exec $command")"
# Match the original pane geometry before swap so tmux cannot reflow and reset
# the copy-mode viewport in split panes.
tmux resize-window -t "$replica" -x "$width" -y "$height"
tmux set-option -p -t "$replica" @tmux-flash-active 1
tmux set-hook -p -t "$replica" 'after-copy-mode[tmux-flash-rs-guard]' \
  'if-shell -F "#{==:#{@tmux-flash-active},1}" "send-keys -X cancel"'
if [[ "$zoomed" == 1 ]]; then
  if tmux swap-pane -Z -s "$replica" -t "$pane" \; \
    select-pane -t "$replica"; then
    :
  else
    # Older tmux versions do not support swap-pane -Z.
    tmux swap-pane -s "$replica" -t "$pane" \; \
      resize-pane -Z -t "$replica" \; \
      select-pane -t "$replica"
  fi
else
  tmux swap-pane -s "$replica" -t "$pane" \; \
    select-pane -t "$replica"
fi
