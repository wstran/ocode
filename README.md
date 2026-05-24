<h1 align="center">opencode</h1>

<p align="center">
  <b><code>ocode</code></b> — read and edit code in your terminal, fast.
</p>

<p align="center">
  <img src="docs/welcome.png" alt="opencode" width="720">
</p>

Open a file or a folder and opencode renders it instantly: syntax-highlighted
code, a file tree, selection and a real clipboard. The whole screen is your
code — the only chrome is one status line at the bottom.

## Features

- **Fast on big files** — rope buffer with incremental highlighting; ~0% CPU idle.
- **Selection & system clipboard** — `Shift`-select, then `Ctrl+C`/`X`/`V`; paste into any app.
- **Real undo/redo** — word-granular; "unsaved" means the text actually differs from disk.
- **Watches the file** — auto-reloads external changes when clean, warns when not.
- **15 themes, 79 languages** — live theme preview; add your own `.tmTheme` or `.sublime-syntax`.

## Install

```sh
git clone https://github.com/wstran/opencode
cd opencode
cargo install --path .        # builds and installs `ocode`
```

Needs **Rust 1.85+**. Pure-Rust dependencies — no C compiler.

## Usage

```sh
ocode               # browse the current directory
ocode src/main.rs   # open a file
ocode ./project     # open a directory (file tree)
ocode --style       # change the color scheme
```

<p align="center">
  <img src="docs/editor.png" alt="opencode editing a Rust file" width="820">
</p>

## Keyboard

`Ctrl` is the command key (a terminal never delivers `Cmd` to an app). Hold
**`Shift`** with any motion to select instead of move.

**Commands**

| Key | Action |
|-----|--------|
| `Ctrl+S` | Save |
| `Ctrl+A` | Select all |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / cut / paste |
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo |
| `Ctrl+F` | Find |
| `Ctrl+B` | File browser |
| `Ctrl+R` | Reload from disk |
| `Ctrl+Q` | Quit |

**Move** (add `Shift` to select)

| Move by… | macOS | Windows / Linux |
|----------|-------|-----------------|
| Character | `←` `→` | `←` `→` |
| Word | `⌥ ←/→` | `Ctrl ←/→` |
| Line (up/down) | `↑` `↓` | `↑` `↓` |
| Block (blank line) | `⌥ ↑/↓` | `Ctrl ↑/↓` |
| Line start / end | `Fn ←/→` (`Home`/`End`) | `Home` `End` |
| File top / bottom | `Ctrl Home` `Ctrl End` | `Ctrl Home` `Ctrl End` |
| Scroll a screen | `Fn ↑/↓` | `PageUp` `PageDown` |

**Edit**

| Key | Action |
|-----|--------|
| any character / paste | Insert — replaces the selection if any |
| `Enter` | New line (keeps indent) |
| `Backspace` / `Delete` | Delete left / right, or the selection |
| `⌥ Delete` · `Ctrl Backspace` | Delete the word behind |
| `⌥ Fn Delete` · `Ctrl Delete` | Delete the word ahead |
| `Tab` / `Shift+Tab` | Indent / outdent the line |

**File tree** (`Ctrl+B`): `↑/↓` move · `→/←` expand/collapse · `Enter`/`Space`
open · `Tab` to editor. Opening a file gives the editor the full screen.

## Terminal setup (macOS)

Enable your terminal's meta key, or `Option`+key just inserts accented
characters instead of reaching opencode (word motion, word delete):

- **iTerm2** — Settings → Profiles → Keys → Key Mappings → Presets → **Natural Text Editing**
- **Terminal.app** — Settings → Profiles → Keyboard → **Use Option as Meta key**
- **Kitty / WezTerm / Ghostty** — works out of the box

## Styles

A scheme recolors only your **code** — your terminal background is untouched.
**15 built in** (`base16-*`, Solarized, Dracula, One Dark, …); drop a `.tmTheme`
into `~/.config/opencode/themes/` for more. Your pick is saved on first launch.

## Languages

**79 languages** including TOML, `.env`, INI and Dockerfile. Add any
`.sublime-syntax` grammar in `~/.config/opencode/syntaxes/`.

## Changes on disk

opencode watches the open file. If another program edits it: a **clean** buffer
auto-reloads (undoable); with **unsaved edits** the status bar warns instead of
overwriting — `Ctrl+R` to reload, or `Ctrl+S` twice to keep yours.

## License

[MIT](LICENSE). Bundled themes and grammars are MIT by their authors (see the
`NOTICE` files). Built with [ratatui](https://github.com/ratatui/ratatui),
[crossterm](https://github.com/crossterm-rs/crossterm),
[ropey](https://github.com/cessen/ropey),
[syntect](https://github.com/trishume/syntect) and
[arboard](https://github.com/1Password/arboard).
