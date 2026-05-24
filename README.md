<h1 align="center">opencode</h1>

<p align="center">
  <b><code>ocode</code></b> — read and edit code in your terminal, fast.
</p>

<p align="center">
  <img src="docs/editor.png" alt="opencode editing a Rust file: file tree, syntax-highlighted code and a status bar" width="820">
</p>

opencode opens a file or a folder and renders it instantly — syntax-highlighted
code, a file tree, smooth scrolling, selection and a real clipboard. The whole
screen is your code; the only chrome is one status line at the bottom.

---

## Contents

- [Features](#features) · [Install](#install) · [Usage](#usage)
- [Keyboard](#keyboard) · [Terminal setup](#terminal-setup) (read this on macOS)
- [Styles](#styles) · [Languages](#languages) · [Changes on disk](#changes-on-disk)
- [How it works](#how-it-works) · [Contributing](#contributing) · [License](#license)

## Features

- **Fast on big files** — a rope buffer with incremental highlighting; near-0%
  CPU while you read.
- **Selection & system clipboard** — `Shift`-select, then `Ctrl+C`/`X`/`V` copy,
  cut and paste through your OS clipboard (paste into any other app). Typing or
  pasting over a selection replaces it.
- **Honest undo/redo** — word-granular, and "unsaved" means the text genuinely
  differs from disk, so undoing back to the saved content counts as clean.
- **Watches the file** — if another editor changes it, a clean buffer reloads
  itself; a dirty one warns instead of clobbering your work.
- **15 color schemes + your own** — live preview; a scheme recolors only the
  *code*, leaving your terminal background untouched.
- **79 languages** out of the box (incl. TOML, `.env`, INI, Dockerfile), plus
  any Sublime grammar you drop in.
- **macOS & Windows/Linux** — one consistent keymap, detected per platform.

## Install

```sh
git clone https://github.com/wstran/opencode
cd opencode
cargo build --release
cp target/release/ocode /usr/local/bin/   # or: cargo install --path .
```

Needs **Rust 1.85+** (edition 2024). Pure-Rust dependencies — no C compiler.

## Usage

```sh
ocode               # browse the current directory
ocode src/main.rs   # open a file straight into the editor
ocode ./project     # open a directory with the file tree focused
ocode --style       # (-s) re-open the style picker and save a new scheme
```

On first run a **style picker** opens (live preview; `↑/↓`, then `Enter`). Your
choice is saved to `~/.config/opencode/config`, so later runs go straight in.

## Keyboard

opencode is **`Ctrl`-based**: a terminal never delivers `Cmd` (`⌘`) to the app —
the terminal keeps it for its own menus — so `Cmd` is not a shortcut here.
`Ctrl` works identically on macOS and PC. Hold **`Shift`** with any motion to
select instead of move.

**Commands** — `Ctrl` + a letter, the same everywhere:

| Save | Select all | Copy / Cut / Paste | Undo / Redo | Find | Files | Reload | Quit |
|------|-----------|--------------------|-------------|------|-------|--------|------|
| `Ctrl+S` | `Ctrl+A` | `Ctrl+C` `Ctrl+X` `Ctrl+V` | `Ctrl+Z` `Ctrl+Y` | `Ctrl+F` | `Ctrl+B` | `Ctrl+R` | `Ctrl+Q` |

**Move / select** — the only keys that differ by platform are word and line
motion (see [Terminal setup](#terminal-setup) so they reach the app on macOS):

| Move by…              | macOS              | Windows / Linux     |
|-----------------------|--------------------|---------------------|
| Character             | `←` `→`             | `←` `→`              |
| Word                  | `⌥ ←` `⌥ →`         | `Ctrl ←` `Ctrl →`    |
| Line (up/down)        | `↑` `↓`             | `↑` `↓`             |
| Block (to blank line) | `⌥ ↑` `⌥ ↓`         | `Ctrl ↑` `Ctrl ↓`   |
| Start / end of line   | `⌘ ←` `⌘ →`  *or*  `Fn ←` `Fn →` | `Home` `End` |
| Top / bottom of file  | `⌥ ↑` / `⌥ ↓` (repeat) | `Ctrl Home` `Ctrl End` |
| Scroll a screen       | `Fn ↑` `Fn ↓`       | `PageUp` `PageDown` |

**Edit:**

| any character / paste | `Enter` | `Backspace` / `Delete` | `⌥/Ctrl + Backspace` | `Tab` / `Shift+Tab` |
|-----------------------|---------|------------------------|----------------------|---------------------|
| insert (replaces selection) | new line, keeps indent | delete left / right, or the selection | delete previous word | indent / outdent line |

**File tree** (`Ctrl+B`): `↑/↓` move · `→/←` expand/collapse · `Enter`/`Space`
open · `Tab` jump to editor. Opening a file closes the tree for a full-screen
editor — `Ctrl+B` again to pick another.

## Terminal setup

> **macOS users — do this once**, or `⌥`/`⌘` motions won't reach opencode.

A terminal decides what `Option` and `Cmd` send. Enable its text-editing preset
so `⌥ ←/→` (word) and `⌘ ←/→` (line start/end) are delivered as real keys:

| Terminal | Setting |
|----------|---------|
| **iTerm2** | Settings → Profiles → Keys → Key Mappings → **Presets… → Natural Text Editing** |
| **Terminal.app** | Settings → Profiles → Keyboard → **Use Option as Meta key** |
| **Kitty / WezTerm / Ghostty** | works out of the box |

With that on, `⌘ ←/→` become start/end of line and `⌥ ←/→` jump by word — the
native macOS feel, no `Fn` needed. (Windows/Linux terminals send `Ctrl`+arrow
and `Home`/`End` directly.)

## Styles

A scheme recolors only your **code** — opencode never paints a background, so
your terminal's own background (and transparency) is preserved. The picker tags
each `dark` / `light` to match your terminal.

**15 built in:** seven from syntect (`base16-*`, `Solarized`, `InspiredGitHub`)
plus **Dracula, One Dark, One Half Dark/Light, Coldark Dark/Cold, Sublime
Snazzy, Two Dark**. Add more by dropping a `.tmTheme` into
`~/.config/opencode/themes/`.

<p align="center">
  <img src="docs/welcome.png" alt="opencode welcome screen" width="660">
</p>

## Languages

**79 languages** — syntect's 75 defaults plus bundled TOML, `.env`, INI and
Dockerfile. Add any `.sublime-syntax` grammar in `~/.config/opencode/syntaxes/`
to support more. List them all with `cargo run --example catalog`.

## Changes on disk

opencode checks the open file ~once a second. If another program edits it:

- **No unsaved edits here** → it **auto-reloads** (flashes *↻ Reloaded*); the
  reload is undoable, so `Ctrl+Z` brings the old version back.
- **You have unsaved edits** → no clobbering: the status bar turns amber —
  *⚠ changed on disk* — and you choose **`Ctrl+R`** to reload (discard yours) or
  **`Ctrl+S` twice** to overwrite with yours.

## How it works

- **`ropey`** rope buffer — edits and large files stay O(log n).
- **Incremental highlighting** — only visible lines are re-highlighted, resuming
  from a cached parser checkpoint.
- **Input-driven loop** — redraws on keypress (≈0% CPU); while a file is open it
  wakes about once a second for a cheap mtime check.
- **O(1) undo snapshots** — cheap rope clones, grouped at word boundaries.

```
src/main.rs   CLI, terminal setup, event loop      src/highlight.rs  syntect + cache
src/app.rs    state & key handling                 src/config.rs     config & user folders
src/buffer.rs rope, cursor, selection, undo, sync   src/ui.rs         rendering
src/tree.rs   lazy file tree                        assets/           themes & grammars
```

## Contributing

Issues and pull requests are welcome. Keep changes focused, run `cargo build`
and `cargo test` before opening a PR, and match the surrounding style.

## License

[MIT](LICENSE). Bundled themes and grammars are MIT by their respective authors
(see `assets/themes/NOTICE` and `assets/syntaxes/NOTICE`). Built with
[ratatui](https://github.com/ratatui/ratatui),
[crossterm](https://github.com/crossterm-rs/crossterm),
[ropey](https://github.com/cessen/ropey),
[syntect](https://github.com/trishume/syntect) and
[arboard](https://github.com/1Password/arboard).
