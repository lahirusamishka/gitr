# rgitk-gui

A git commit graph viewer with smooth, curved lane transitions (the
GitKraken/Sourcetree look), built with Rust + egui/eframe. This is the
GUI sibling of the terminal `rgitk` — same commit-graph/lane algorithm,
but drawn with cubic-bezier curves instead of ASCII, since a real curve
needs pixel graphics, not a terminal grid.

## Build

Needs a reasonably recent Rust (1.75+; the same MSRV note from the
terminal version's README applies — if your distro's `cargo`/`rustc` is
too old, install a newer versioned package).

You'll also need the usual Linux GUI dev libraries for windowing/OpenGL
(these are almost certainly already on a normal desktop, this is only
relevant on a minimal/server image):

```sh
# Debian/Ubuntu
sudo apt install libxkbcommon-dev libx11-dev libxrandr-dev libxi-dev \
                  libxcursor-dev libxinerama-dev libgl1-mesa-dev pkg-config
```

Then:

```sh
cargo build --release
```

The binary is at `target/release/rgitk-gui`. Run it directly, or copy it
onto your `$PATH`:

```sh
sudo cp target/release/rgitk-gui /usr/local/bin/rgitk-gui
```

## Use

```sh
rgitk-gui                # open the repo in the current directory
rgitk-gui /path/to/repo  # open a specific repo
rgitk-gui --current      # only the current branch, not all refs
rgitk-gui --limit 200    # cap how many commits are loaded
```

- Click any commit to select it — details show in the right-hand panel.
- **Full diff** / **Stat only** buttons run `git show` for the selected
  commit and display it in a scrollable pane.
- Search box + **Find next** (or press Enter) jumps to the next commit
  whose message, author, or hash matches.
- **Refresh** re-reads refs/commits (e.g. after a `git fetch`).

## Notes

- The lane graph uses the same lane-assignment logic as the terminal
  version: each commit gets a lane, curves show branch/merge points,
  and the curve is drawn over a single row's height (like the reference
  screenshots of GitKraken-style graphs) rather than a full multi-row
  smooth path — this keeps the renderer simple and fast even on large
  histories.
- Rendering draws the whole loaded history each frame (immediate-mode
  GUI); a few thousand commits is fine, tens of thousands may get sluggish
  — use `--limit` on very large repos.
