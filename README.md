# tmux-flash

An independently installable TPM plugin that searches the visible copy-mode
viewport and assigns jump labels to matches. The Rust binary is distributed as
prebuilt GitHub Release assets; users do not need Cargo or a Rust toolchain.

The repository is named `tmux-flash-rs` while it is developed separately from
the Go implementation. Its TPM source slug is `tmux-flash-rs`; the plugin,
executable, release assets, and download cache keep the public `tmux-flash`
name.

## Install

```tmux
set -g @plugin 'wheregrassdreams/tmux-flash-rs'
```

Install through TPM with `prefix + I`, then reload tmux. On plugin load, the
entrypoint downloads the asset matching the OS and CPU architecture into
`${XDG_CACHE_HOME:-~/.cache}/tmux-flash/<version>/<target>`. The installer
verifies the published SHA-256 file before extraction. If the repository is
private, authenticate with `gh auth login`; public releases can be fetched with
`curl` alone.

For a manual installation, clone the repository and source its TPM entrypoint:

```sh
git clone https://github.com/wheregrassdreams/tmux-flash-rs ~/.tmux/plugins/tmux-flash-rs
```

```tmux
run-shell ~/.tmux/plugins/tmux-flash-rs/tmux-flash.tmux
```

Set any `@flash-*` options before the `run-shell` line so the entrypoint can
install the configured bindings.

Supported release targets are macOS arm64 and x86_64, and Linux arm64 and
x86_64.

## Usage

With the default binding, set `mode-keys vi`, enter copy mode with `prefix + [`,
then press `s` and type a search pattern. You can also bind keys in the prefix
or root table to start Flash from a normal pane; Flash enters copy mode before
capturing it. Press a displayed label to jump to that match, or Enter to jump
to the nearest match. Backspace edits the query; Escape or Ctrl-C cancels and
leaves the pane in copy mode. To extend an existing selection, press `v` before
starting Flash, then use the label and `y` to yank the selected range.

Flash searches the visible pane viewport. If the pane's dimensions actually
change while Flash is active, the search is cancelled; a resize signal with no
dimension change does not cancel it. Entering Flash from a zoomed pane keeps the
pane zoomed, and restores that zoom state when Flash exits.

While flash is active, a pane-local tmux hook immediately exits any copy mode
started on the flash pane. This works regardless of the key or mouse action
used to request copy mode. The hook uses its own array entry, so other
`after-copy-mode` hooks on the pane remain installed and run as usual.

`@flash-copy-mode-vi-keys` selects one or more keys in the vi copy-mode table
(default `s`). `@flash-prefix-keys` selects keys in the prefix table; press the
configured tmux prefix, then one of these keys. `@flash-root-keys` selects keys
in the root table, which trigger without a prefix and can intercept keys from
applications inside panes. All three accept space-separated key names. The
prefix and root lists are empty by default.

`@flash-labels` controls the label alphabet and must contain unique, printable,
non-space ASCII characters.

Colors are configured independently with `@flash-color-backdrop`,
`@flash-color-match-fg`, `@flash-color-match-bg`, `@flash-color-current-fg`,
`@flash-color-current-bg`, `@flash-color-label-fg`, `@flash-color-label-bg`,
`@flash-color-status`, and `@flash-color-status-error`. Each value accepts
`#RRGGBB` (the leading `#` is optional).

```tmux
set -g @flash-copy-mode-vi-keys 's C-s'
set -g @flash-prefix-keys 'f'
# set -g @flash-root-keys 'M-f'
set -g @flash-labels 'asdfghjklqwertyuiopzxcvbnm'
set -g @flash-color-match-bg '#385d8a'
set -g @flash-color-current-bg '#23734f'
```

## Unicode behavior

Search operates on grapheme clusters. Match proximity and jump positions use
terminal cell columns, so CJK characters and emoji do not shift the selected
location. Jump labels are restricted to single-column printable ASCII so each
label maps to one keypress and one terminal cell.

## Inspiration

[tmux-flash](https://github.com/AndreVicencio/tmux-flash) is the direct
reference for this project's tmux workflow. Its search-and-label interaction
traces back to [flash.nvim](https://github.com/folke/flash.nvim), the origin of
the Flash interaction model.

## Release process

Push a semantic version tag such as `vX.Y.Z` to `wheregrassdreams/tmux-flash-rs`.
The release workflow builds the four supported targets, archives each
`tmux-flash` binary, writes a SHA-256
sidecar, and attaches both files to the GitHub Release. The TPM installer picks
the matching archive and verifies its checksum before caching the executable.

For local development:

```sh
cargo test --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

On the development Mac, use the Command Line Tools linker if the active Nix
clang toolchain cannot resolve macOS `libiconv`:

```sh
CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER="$(xcrun --find clang)" cargo test --locked
CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER="$(xcrun --find clang)" cargo clippy --locked --all-targets -- -D warnings
CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER="$(xcrun --find clang)" cargo build --locked --release
```

This is a local toolchain workaround only. Release builds use the standard
GitHub-hosted runner toolchains and do not need this setting.

## Requirements

- tmux 3.1 or newer
- `mode-keys vi` when using `@flash-copy-mode-vi-keys` (the default binding)
- For a private GitHub repository, GitHub CLI authentication for first install

The plugin has no dependency on tmux-config or other tmux plugins.
