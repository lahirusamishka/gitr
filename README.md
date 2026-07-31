# gitr — a compact git commit graph viewer

Alternative to **gitk**, inspired by **VS Code Git Graph**.  
Runs from any git repo. Type `gr` and see your commit graph.

## Quick start

```sh
make build && make install   # builds gitr, installs as `gr` command
gr                           # open current repo
```

From any git repo root, just run `gr`. That's it.

## Controls

| Key | Action |
|---|---|
| `gr` | Launch from any git repo |
| `Ctrl+Q` | Exit |
| Click a row | Select commit, show diff in right panel |
| Search + Enter | Find commits by message/author/hash |

## Features

- **Compact graph** — branch lanes recycle automatically, no pyramid effect
- **Side-by-side diff** — meld-style viewer with red/green/blue highlighting
- **Ref pills** — HEAD, branches, and tags shown as rounded badges
- **Aligned metadata** — author, date, hash columns (like VS Code Git Graph)
- **Curved bezier lanes** — smooth S-curves for branch/merge transitions
- **Dark theme** — Catppuccin Mocha palette

## Install

```sh
make build && make install
```

This builds the binary and copies it to `/usr/local/bin/gr`.

To uninstall: `make uninstall`.

Or manually:

```sh
cargo build --release
sudo cp target/release/gitr /usr/local/bin/gr
```

### Dependencies (Linux)

```sh
sudo apt install libxkbcommon-dev libx11-dev libxrandr-dev libxi-dev \
                 libxcursor-dev libxinerama-dev libgl1-mesa-dev pkg-config
```

## Usage

```sh
gr                # open repo in current directory
gr /path/to/repo  # open a specific repo
gr --current      # only the current branch, not all refs
gr --limit 200    # cap how many commits are loaded
```

## Support

If you find this useful, [buy me a coffee ☕](https://buymeacoffee.com/samishka)
