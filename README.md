<h1 align="center">opencode</h1>

<p align="center">
  <b><code>ocode</code></b> — a fast, no-frills code reader &amp; editor that lives in your terminal.
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <img alt="Rust 1.85+" src="https://img.shields.io/badge/rust-1.85%2B-orange.svg">
  <img alt="Platforms" src="https://img.shields.io/badge/platform-macOS%20·%20Linux%20·%20Windows-lightgrey.svg">
</p>

Open a file or a folder and it renders instantly — syntax-highlighted code, a
file tree, smooth scrolling, and nothing in the way. The whole screen is your
code; the only chrome is a single status line at the bottom.

```text
ocode <path>        # open a file or directory (defaults to the current dir)
```

## Highlights

- ⚡ **Smooth on big files** — a rope buffer and incremental highlighting; it
  idles at 0% CPU while you read.
- 🎨 **15 built-in styles + your own** — live preview, your choice is saved, and a
  style only recolors *code* so your terminal background stays untouched.
- 🗂️ **79 languages out of the box** — including TOML, `.env`, INI and Dockerfile;
  drop in any Sublime grammar to add more.
- 🌳 **File browser** — one key (`Ctrl+B`) opens the working folder, jumps back to
  it, and hides it again.
- ⌨️ **Mac &amp; PC keymaps, auto-detected** — no `Cmd` needed; works in any terminal.
- 🧠 **Real undo/redo** — word-granular, and "unsaved" means the text actually
  differs from disk (undo back to saved → clean).

## Install

```sh
git clone https://github.com/wstran/opencode
cd opencode
cargo build --release
cp target/release/ocode /usr/local/bin/   # or anywhere on your $PATH
```

Needs **Rust 1.85+** (edition 2024). Pure-Rust dependencies only — no C compiler
required.

## Usage

```sh
ocode               # browse the current directory
ocode src/main.rs   # open a single file straight into the editor
ocode ./project     # open a directory with the file tree focused
```

## Styles

**First launch** opens a **style picker**: the list of color schemes on the left
and a **live preview** of real code on the right. Pick one with `↑/↓` and press
`Enter` — your choice is **saved**, so every launch after that goes straight to
the editor.

```
↑ / ↓   select        Enter   apply & save        Esc   quit
```

To change it later:

```
ocode --style          # (or -s) re-open the picker and pick a new scheme
```

The selection is stored in `~/.config/opencode/config`
(`$XDG_CONFIG_HOME/opencode/config` if set).

### Backgrounds are never touched

A style only recolors your **code** — opencode never paints a background, so your
terminal's own background (and any transparency) is preserved exactly. That means
light schemes look best on a light terminal and dark schemes on a dark one; the
picker tags each as `dark` / `light` to help you choose.

### 15 schemes built in

Seven from syntect (`base16-*`, `Solarized`, `InspiredGitHub`) plus eight bundled
community schemes: **Dracula, One Dark, One Half Dark/Light, Coldark Dark/Cold,
Sublime Snazzy, Two Dark** (MIT — see `assets/themes/NOTICE`).

### Add your own

Drop any `.tmTheme` file into `~/.config/opencode/themes/` and it appears in the
picker automatically — no limit.

## Keybindings

The screen shows only code; these are the controls. opencode **auto-detects your
OS** and accepts the modifier that platform actually delivers — `Option` (`⌥`)
for word motion on macOS, `Ctrl` for the same on Windows/Linux. Everything else
is `Ctrl`, which every terminal forwards on both.

> **No `Cmd` (`⌘`):** macOS terminals capture `Cmd` for themselves and never pass
> it to terminal apps, so opencode binds `Ctrl`/`⌥` instead — the portable choice.

### Global

| Key                 | Action                                          |
|---------------------|-------------------------------------------------|
| `Ctrl+S`            | Save the current file (flashes a green ✓ Saved) |
| `Ctrl+Z`            | Undo                                            |
| `Ctrl+Y` / `Ctrl+Shift+Z` | Redo                                      |
| `Ctrl+B`            | File browser: open & focus → back to it → hide  |
| `Ctrl+F`            | Search in the current file                      |
| `Ctrl+Q` / `Ctrl+C` | Quit (press twice if there are unsaved changes) |

> **Why `Ctrl+Z`, not `⌘Z` on macOS?** Terminals never deliver `Cmd` to the app
> (the terminal app keeps it). So undo/redo — and every shortcut — use `Ctrl`,
> which works identically on macOS and PC.

### Browsing & opening files

`Ctrl+B` is a three-step cycle, so the **same key opens the file list and brings
you back to it**:

1. From the editor → the working folder opens on the left, focused.
2. Pick a file with `↑/↓` then **`Enter` or `Space`**; you land in the editor
   (the list stays open). On a folder, `Enter`/`Space` expands/collapses it.
3. Press `Ctrl+B` again → focus jumps **back to the list** to pick another file.
4. Press it once more (while in the list) → the list hides, full width for code.

`Tab` also flips focus between the editor and the list whenever it's open.
Switching files with **unsaved edits** is guarded: the first `Enter`/`Space`
warns; press `Ctrl+S` to save or confirm again to discard and open.

Before any file is open, the screen shows a centered **opencode** welcome logo;
it disappears the moment a file loads.

### Editor — navigation

The "jump" modifier is **`⌥` (Option) on macOS** and **`Ctrl` on Windows/Linux** —
exactly one per platform, so every combo maps to a single action (no two
shortcuts do the same thing). Four granularities:

| Motion                  | Stops at                                  | macOS      | Win/Linux    |
|-------------------------|-------------------------------------------|------------|--------------|
| **Sub-token** (fine)    | `.` `:` `;`, underscores, camelCase humps | `⇧ ←/→`     | `Shift ←/→`   |
| **Word** (medium)       | word boundaries                           | `⌥ ←/→`     | `Ctrl ←/→`    |
| **WORD** (coarse, fast) | whitespace only                           | `⌥⇧ ←/→`    | `Ctrl⇧ ←/→`   |
| **Block** (vertical)    | the previous / next blank line            | `⌥ ↑/↓`     | `Ctrl ↑/↓`    |

`getUserName.id` under sub-token splits to `get|User|Name|.|id`; under WORD it's
one leap to the next space. (Motions sit on **arrow keys**, not punctuation —
terminals deliver `Shift/Option/Ctrl + arrow` reliably, but not `Option + < >`.)

### Editor — everything else (same on both)

| Key                       | Action                                  |
|---------------------------|-----------------------------------------|
| Arrows                    | Move the cursor                         |
| `Ctrl+A` / `Ctrl+E`       | Start / end of line                     |
| `Ctrl+Home` / `Ctrl+End`  | Start / end of file                     |
| `PageUp` / `PageDown`     | Scroll a screen                         |
| `Tab` / `Shift+Tab`       | Indent / outdent the current line       |
| `Enter`                   | New line (keeps the current indent)     |
| `Backspace` / `Ctrl+H`    | Delete before the cursor                |
| `Delete` / `Ctrl+D`       | Delete after the cursor                 |
| `⌥/Ctrl + Backspace`      | Delete the previous word                |
| any character             | Insert it                               |

### MacBook keyboard (no Home / End / PageUp / Delete keys)

A MacBook has none of those keys as dedicated keys — they're `Fn + arrow`
combos. opencode is built so you **never need them**; every action has a
`Ctrl`-letter or `Option`-arrow path that needs no `Fn`:

| You want            | No-Fn key (works on MacBook)        | Dedicated key (PC / Fn) |
|---------------------|-------------------------------------|-------------------------|
| Start / end of line | `Ctrl+A` / `Ctrl+E`                 | `Home` / `End`          |
| Forward-delete      | `Ctrl+D`                            | `Delete` / `Fn+Delete`  |
| Big jump up / down  | `⌥ ↑` / `⌥ ↓` (block; hits doc ends)| `Ctrl+Home`/`End`, `PgUp`/`PgDn` |
| Word back-delete    | `⌥ Backspace`                       | `Ctrl+Backspace`        |

The dedicated keys still work everywhere (on a MacBook via `Fn`), they're just
optional. PC keyboards have all of them, so nothing is lost there either.

> Whether a terminal forwards modified arrows depends on the terminal: most send
> `Shift/Ctrl + arrow`; `Option + arrow` on macOS often needs **"Use Option as
> Meta key"** (Terminal/iTerm) or works out of the box on Kitty/WezTerm/Ghostty.
> The `Ctrl`-letter motions above always work, on every terminal and keyboard.

### File tree (when focused)

| Key       | Action                                            |
|-----------|---------------------------------------------------|
| `↑` / `↓` | Move selection                                    |
| `→`       | Expand a directory                                |
| `←`       | Collapse a directory                              |
| `Enter` / `Space` | Open a file / toggle a directory          |
| `Tab`     | Jump to the editor                                |

### Search (after `Ctrl+F`)

| Key     | Action                          |
|---------|---------------------------------|
| type    | Build the query                 |
| `Enter` | Jump to the next match (wraps)  |
| `Esc`   | Close search                    |

## Saving & quitting

- **Saving is manual** — `Ctrl+S`. There is no auto-save: opencode never writes
  your file behind your back. A successful save flashes a green **✓ Saved** in
  the status bar for ~1.5 s, then clears itself.
- **Quitting** is `Ctrl+Q` (or `Ctrl+C`).
- **Quitting with unsaved changes** does **not** save and does **not** silently
  discard. The first `Ctrl+Q` is refused with a warning —
  *"Unsaved changes — Ctrl+Q again to quit, or Ctrl+S to save"* — so you choose:
  press `Ctrl+S` to save, or `Ctrl+Q` again to quit and **discard** the changes.
- **"Unsaved" means the text actually differs from what's on disk.** If you edit
  and then undo (or delete) back to the saved content, the buffer is considered
  clean again — quitting and switching files no longer prompt. The same applies
  after `Ctrl+B` / `Enter` to another file.

## Supported languages

Syntax highlighting covers **79 languages**: syntect's 75 defaults plus four
bundled grammars — **TOML, `.env` (DotENV), INI/cfg/conf, Dockerfile**. The
common ones:

```
C, C++, C#, Go, Rust, Java, Scala, JavaScript, Python, Ruby, PHP, Perl, Lua,
Haskell, Clojure, Lisp, OCaml, Erlang, Pascal, D, R, MATLAB, Shell/Bash, Batch,
Makefile, SQL, HTML, CSS, XML, JSON, YAML, Markdown, LaTeX, Diff, Graphviz, Tcl,
Groovy, Objective-C/C++, reStructuredText, TOML, .env, INI, Dockerfile, …
```

Print the exact list with extensions: `cargo run --example catalog`.

**Add more** — drop any `.sublime-syntax` grammar into
`~/.config/opencode/syntaxes/` and it loads at startup (this is how you'd add
TypeScript, Kotlin, Swift, etc.).

## How it stays smooth

- **ropey** rope buffer — large files edit in O(log n), not O(n).
- **Incremental highlighting** — only visible lines are highlighted, resuming
  from a cached parser checkpoint; an edit re-highlights only from that line
  down.
- **Input-driven loop** — redraws happen on keypress, so it sits at 0% CPU while
  you read; the only timed wake-up is to fade the green save flash.
- **O(1) undo snapshots** — undo/redo store cheap rope clones; steps break at
  word boundaries, cursor jumps and structural edits, so `Ctrl+Z` walks back
  word by word rather than wiping a whole session at once.

## Notes & limits

- Tabs in a file render as a single column so the cursor stays aligned.
- Undo history is per session and capped (not persisted across launches).
- One file open at a time (no split panes / tabs yet).
- The status bar at the very bottom shows the path and `Ln, Col` — that is the
  only UI line; everything above it is your code.

## Project layout

```
src/
  main.rs        CLI parsing, terminal setup/teardown, event loop
  app.rs         application state and key handling
  buffer.rs      rope text buffer, cursor, edits, undo, search
  tree.rs        lazy file-tree model
  highlight.rs   syntect integration + incremental highlight cache
  config.rs      saved style + user theme/grammar folders
  ui.rs          rendering (welcome, picker, tree, editor, status)
assets/
  themes/        bundled .tmTheme color schemes (+ NOTICE)
  syntaxes/      bundled .sublime-syntax grammars (+ NOTICE)
```

Run the test suite with `cargo test`; list bundled languages and themes with
`cargo run --example catalog`.

## Contributing

Issues and pull requests are welcome. Please keep changes focused, run
`cargo build` and `cargo test` before opening a PR, and match the existing style.

## License

[MIT](LICENSE) © Wilson Tran.

Bundled color schemes and grammars are MIT-licensed by their respective authors;
see `assets/themes/NOTICE` and `assets/syntaxes/NOTICE` for attribution.

## Acknowledgements

Built with [ratatui](https://github.com/ratatui/ratatui),
[crossterm](https://github.com/crossterm-rs/crossterm),
[ropey](https://github.com/cessen/ropey) and
[syntect](https://github.com/trishume/syntect).
