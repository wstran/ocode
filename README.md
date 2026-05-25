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
- **Images & binaries** — views PNG/JPEG/… inline in Ghostty; other binaries show a hex preview, never an error.
- **15 themes, 79 languages** — live theme preview; add your own `.tmTheme` or `.sublime-syntax`.

## Install

```sh
git clone https://github.com/wstran/opencode
cd opencode
cargo install --path .        # builds and installs `ocode`
```

Needs **Rust 1.85+**. Pure-Rust dependencies — no C compiler.

> **Recommended terminal: [Ghostty](https://ghostty.org).** It gives the
> smoothest keys and inline image preview — see [Ghostty setup](#ghostty-setup).

## Usage

```sh
ocode               # browse the current directory
ocode src/main.rs   # open a file
ocode ./project       # open a directory (file tree)
ocode --style         # change the color scheme
ocode --ghostty-config # print the recommended Ghostty config (below)
```

<p align="center">
  <img src="docs/editor.png" alt="opencode editing a Rust file" width="820">
</p>

## Keyboard

`Ctrl` is the command key (terminals don't deliver `Cmd` shortcuts to an app;
`⌘←/→` is wired up via [Ghostty setup](#ghostty-setup)). Hold **`Shift`** with
any motion to select instead of move.

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
| `Esc` | Step back: clear selection → leave tree → open file list & arm quit → quit |
| `Ctrl+Q` | Quit |

**Move** (add `Shift` to select)

| Move by… | macOS | Windows / Linux |
|----------|-------|-----------------|
| Character | `←` `→` | `←` `→` |
| Word | `⌥ ←/→` | `Ctrl ←/→` |
| Line (up/down) | `↑` `↓` | `↑` `↓` |
| Block (blank line) | `⌥ ↑/↓` | `Ctrl ↑/↓` |
| Line start / end | `⌘ ←/→`¹ · `Fn ←/→` (`Home`/`End`) | `Home` `End` |
| File top / bottom | `Ctrl Home` `Ctrl End` | `Ctrl Home` `Ctrl End` |
| Scroll a screen | `Fn ↑/↓` | `PageUp` `PageDown` |

¹ `⌘ ←/→` line motion needs the one-time [Ghostty setup](#ghostty-setup) below.

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

## Ghostty setup

opencode is built for **[Ghostty](https://ghostty.org) — the recommended
terminal.** It speaks the kitty graphics protocol (inline images, no setup) and
lets you fix the two macOS keys that no terminal can send cleanly. Open your
Ghostty config with **`⌘,`** (its real path differs per OS — on macOS it lives
under *Application Support*, not `~/.config`), paste in the block below, and
reload with `⌘⇧,`. Run **`ocode --ghostty-config`** to print it — it ships in
the binary, so it's there again after any reinstall, no need to remember it:

```ini
# ⌘←/→ → line start / end — i.e. exactly like Fn ←/→ (Home / End).
# Ghostty sends Ctrl+A / Ctrl+E for these by default, which an app can't tell
# apart from a real Ctrl+A (Select-all); this remaps them to Home / End.
keybind = cmd+left=csi:H
keybind = cmd+right=csi:F
keybind = shift+cmd+left=csi:1;2H
keybind = shift+cmd+right=csi:1;2F

# ⌥←/→ → jump by word.
macos-option-as-alt = true
```

So `⌘←/→` ends up doing the same thing as `Fn ←/→`: jump to line start / end
(add `Shift` to select to there).

### Optional — use ⌥ as the command key

Terminals can't deliver `⌘`+letter shortcuts, so opencode's commands live on
`Ctrl`. If you'd rather reach for `⌥`+letter, map each to its control byte
(needs `macos-option-as-alt = true`, above):

```ini
# save · select-all · copy · cut · paste · undo · redo · find · browser ·
# reload · quit  (Ghostty has no same-line comments, so keep them above)
keybind = alt+s=text:\x13
keybind = alt+a=text:\x01
keybind = alt+c=text:\x03
keybind = alt+x=text:\x18
keybind = alt+v=text:\x16
keybind = alt+z=text:\x1a
keybind = alt+y=text:\x19
keybind = alt+f=text:\x06
keybind = alt+b=text:\x02
keybind = alt+r=text:\x12
keybind = alt+q=text:\x11
```

`⌥←/→` stays word-motion — it rides the arrows, not the letters. (`⌥F`/`⌥B`
above take those two letters over from their word-jump aliases; `⌥←/→` is
unaffected.) **Ghostty has no inline comments** — a `#` must be on its own
line, or it becomes part of the keybind.

> **Other terminals** send modified keys differently, so `⌘←/→` line motion and
> inline images are Ghostty-only. For word motion elsewhere, enable a meta key:
> iTerm2 → *Settings → Profiles → Keys → Presets → Natural Text Editing*;
> Terminal.app → *Use Option as Meta key*.

## Styles

A scheme recolors only your **code** — your terminal background is untouched.
**15 built in** (`base16-*`, Solarized, Dracula, One Dark, …); drop a `.tmTheme`
into `~/.config/opencode/themes/` for more. Your pick is saved on first launch.

## Languages

**79 languages** including TOML, `.env`, INI and Dockerfile. Add any
`.sublime-syntax` grammar in `~/.config/opencode/syntaxes/`.

## Images & other files

Open an image — PNG, JPEG, GIF, BMP or WebP — and opencode draws it **inline**,
scaled to fit, using Ghostty's kitty graphics protocol. Any other binary (PDF,
archives, fonts, …) shows a labelled **hex preview** of its first bytes instead
of failing to open.

## Changes on disk

opencode watches the open file. If another program edits it: a **clean** buffer
auto-reloads (undoable); with **unsaved edits** the status bar warns instead of
overwriting — `Ctrl+R` to reload, or `Ctrl+S` twice to keep yours.

## License

[MIT](LICENSE). Bundled themes and grammars are MIT by their authors (see the
`NOTICE` files). Built with [ratatui](https://github.com/ratatui/ratatui),
[crossterm](https://github.com/crossterm-rs/crossterm),
[ropey](https://github.com/cessen/ropey),
[syntect](https://github.com/trishume/syntect),
[arboard](https://github.com/1Password/arboard) and
[image](https://github.com/image-rs/image).
