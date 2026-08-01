# gitr — a compact git commit graph viewer

Alternative to **gitk**, inspired by **VS Code Git Graph**.  
Runs from any git repo. See your commit graph, diffs, branches, and working tree changes all in one window.

## Quick start

```sh
make build && make install   # builds gitr, installs as `gr` command
gr                           # open current repo
```

From any git repo root, just run `gr`.

## Download

Get the latest build from the [Releases page](https://github.com/lahirusamishka/gitr/releases):

| Asset | Description |
|---|---|
| `gitr-x86_64.AppImage` | Portable AppImage — just download, `chmod +x`, and run |
| `gitr-linux-x86_64.tar.gz` | Plain binary tarball |

### AppImage (recommended)

```sh
chmod +x gitr-x86_64.AppImage
./gitr-x86_64.AppImage
```

## Features

- **Compact graph** — branch lanes recycle automatically, no pyramid effect
- **Side-by-side diff** — meld-style viewer with red/green/blue highlighting
- **Ref pills** — HEAD, branches, and tags shown as rounded badges
- **Working tree awareness** — staged/unstaged changes shown as a virtual branch with yellow/red indicators
- **Stash support** — stashes appear as special rows in the graph (mauve nodes)
- **Branch management** — right-click a branch pill to checkout, rename, or delete
- **Live auto-reload** — detects `git add`, `git commit`, `git checkout`, `git push`, and file edits automatically
- **Aligned metadata** — author, date, hash columns
- **Curved bezier lanes** — smooth S-curves for branch/merge transitions
- **Dark theme** — Catppuccin Mocha palette

## Controls

| Key | Action |
|---|---|
| `gr` | Launch from any git repo |
| `Ctrl+Q` / `Ctrl+C` / `Esc` | Exit |
| `Ctrl+R` | Refresh |
| Click a row | Select commit, show diff in right panel |
| Right-click a branch pill | Checkout / Rename / Delete |
| Search + Enter | Find commits by message/author/hash |

## Install from source

### Dependencies (Linux)

```sh
sudo apt install libgtk-3-dev libssl-dev pkg-config \
                 libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
                 libxkbcommon-dev libfontconfig1-dev fuse libfuse2
```

### Build & install

```sh
make build && make install
```

This builds the binary and copies it to `/usr/local/bin/gr`.

To uninstall: `sudo rm /usr/local/bin/gr`.

Or manually:

```sh
cargo build --release
sudo cp target/release/gitr /usr/local/bin/gr
```

## Usage

```sh
gr                # open repo in current directory
gr /path/to/repo  # open a specific repo
gr --current      # only the current branch, not all refs
gr --limit 200    # cap how many commits are loaded
```

## About

gitr (r = Rust) was inspired by **gitg** (GNOME Git graphical interface) and **gitk**.  
It brings together the visual tree of gitg and the quick terminal accessibility of gitk, making it easy to browse your repository and check file diffs at a glance.

Fully open source.

## Support

If you find this useful, [buy me a coffee ☕](https://buymeacoffee.com/lahirusamishka)
