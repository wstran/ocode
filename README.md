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
- **15 themes, 118 languages** — TypeScript + a deep web3/ZK stack, mobile (Swift, Kotlin, Dart), HDL (Verilog, VHDL), Assembly, WebAssembly, plus the usual web/AI/DevOps; add your own `.tmTheme` or `.sublime-syntax`.

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
| `Ctrl+F` | Find — seeds from the selection, highlights every match, `Enter` jumps to the next |
| `Ctrl+B` | File browser |
| `Ctrl+R` | Reload from disk |
| `Esc` | Clear selection; else arm quit (when editing a file it also opens the browser); again to quit |
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
| `Ctrl+K` | Delete the whole line |
| `Ctrl+/` | Toggle comment on the line or selection |
| `Tab` / `Shift+Tab` | Indent / outdent the line, or every line the selection covers |

**File tree** — open it with `Enter` (from the welcome screen) or `Ctrl+B`:
`↑/↓` move · `→/←` expand/collapse · `Enter`/`Space` open · `Tab` to editor. It
re-reads the folder as files appear or disappear. opencode starts on the
welcome screen; opening a file gives the editor the full screen.

## Terminal setup (macOS)

For word motion (`⌥ ←/→`) and word delete, enable your terminal's meta key —
otherwise `⌥`+key just inserts accented characters:

- **iTerm2** — Settings → Profiles → Keys → Presets → **Natural Text Editing**
- **Terminal.app** — Settings → Profiles → Keyboard → **Use Option as Meta key**
- **Ghostty / Kitty / WezTerm** — works out of the box

Inline image preview needs a terminal that speaks the **kitty graphics
protocol** (Ghostty, Kitty).

## Styles

A scheme recolors only your **code** — your terminal background is untouched.
**15 built in** (`base16-*`, Solarized, Dracula, One Dark, …); drop a `.tmTheme`
into `~/.config/opencode/themes/` for more. Your pick is saved on first launch.

## Languages

**118 languages** out of the box. Highlights:

- **TypeScript** (`.ts/.mts/.cts`); `.tsx/.jsx` use a JSX-aware grammar that
  highlights elements and attributes; `.mjs/.cjs` open as JavaScript.
- **Schemas / config / infra**: Protocol Buffers, GraphQL, TOML, `.env`, INI,
  Dockerfile, HCL/Terraform, Nix, Bicep.
- **Web3 / ZK**: Solidity, Vyper, Yul, Huff, Move, Cairo, Noir, Circom,
  ZoKrates, Sway, Cadence, Leo, Lean — plus Tact, FunC (TON), Clarity (Stacks),
  Aiken (Cardano), LIGO (Tezos), Pact (Kadena), TEAL (Algorand), Scilla.
- **Mobile / desktop**: Swift, Kotlin (`.kt/.kts`), Dart, F#, PowerShell.
- **HDL / asm / wasm**: Verilog/SystemVerilog, VHDL, Assembly (NASM/AT&T),
  WebAssembly text (`.wat/.wast`).
- **Modern systems / AI**: Zig, Elixir, Julia.
- **Web supersets** (alias): `.vue/.svelte/.astro → HTML`, `.scss/.sass/.less
  → CSS`, `.mdx → Markdown`.

Add any `.sublime-syntax` grammar in `~/.config/opencode/syntaxes/`.

## Images & other files

Open an image — PNG, JPEG, GIF, BMP or WebP — and opencode draws it **inline**,
scaled to fit, via the kitty graphics protocol (Ghostty, Kitty). Any other
binary (PDF, archives, fonts, …) shows a labelled **hex preview** of its first
bytes instead of failing to open.

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
