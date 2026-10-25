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

Supported release targets are macOS arm64 and x86_64, and Linux arm64 and
x86_64.

## Usage

Set `mode-keys vi`, enter copy mode with `prefix + [`, then press `s` and type a
search pattern. Press a displayed label to jump to that match, or Enter to jump
to the nearest match. Backspace edits the query; Escape or Ctrl-C cancels. To
extend an existing selection, press `v` before `s`, then use the label and `y`
to yank the selected range.

While flash is active, a pane-local tmux hook immediately exits any copy mode
started on the flash pane. This works regardless of the key or mouse action
used to request copy mode. The hook uses its own array entry, so other
`after-copy-mode` hooks on the pane remain installed and run as usual.

`@flash-key` selects one trigger key (default `s`). `@flash-keys` selects
multiple trigger keys and accepts a space- or comma-separated list; when set,
it takes precedence over `@flash-key`. `@flash-labels` controls the label
alphabet. `@flash-colors` accepts comma-separated `name=#RRGGBB` values for
`backdrop`, `match-fg`, `match-bg`, `current-fg`, `current-bg`, `label-fg`,
`label-bg`, `status`, and `status-error`.

```tmux
set -g @flash-key 's'
# set -g @flash-keys 's C-s M-s'
set -g @flash-labels 'asdfghjklqwertyuiopzxcvbnm'
```

## Unicode behavior

Search and labels operate on grapheme clusters. Match proximity and jump
positions use terminal cell columns, so CJK characters and emoji do not shift
the selected location. Tests cover CJK text, emoji including joined sequences,
combining marks, ASCII smart-case matching, and label assignment.

## Release process

Push a semantic version tag such as `v0.1.0` to `wheregrassdreams/tmux-flash-rs`.
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
- `mode-keys vi`
- For a private GitHub repository, GitHub CLI authentication for first install

The plugin has no dependency on tmux-config or other tmux plugins.
