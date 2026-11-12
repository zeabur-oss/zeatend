# Multilingual TUI proof of concept

A throwaway look at the Zeatend install screens (Server, Access, Review, Install, plus a language
picker) in English, 繁體中文 and 简体中文. No SSH, no K3s, no install logic: the server
facts are mock data and the install is a fake 3-minute timeline that runs ten times faster.

```sh
nix develop -c cargo run -p zeatend-tui-poc                      # language from LC_ALL, LC_MESSAGES, LANG
nix develop -c cargo run -p zeatend-tui-poc -- --lang zh-TW      # force a language
nix develop -c cargo run -p zeatend-tui-poc -- --ascii           # act like the Linux console
nix develop -c cargo run -p zeatend-tui-poc -- --shots poc-shots # write every screen as SVG
```

Keys: Up/Down, Tab, Enter, Esc, F2 opens the language picker on any screen, Ctrl+C quits.

## Choices

- **ratatui + crossterm.** The Epic's docs plan (SVGs rendered from `TestBackend`) is ratatui's
  test backend, and crossterm is its default backend. Nothing in the PoC needed anything else.
- **Fluent** (`fluent-bundle`) for strings. Plurals follow CLDR per language and the translator
  owns the sentence (selectors, word order), which gettext's `Plural-Forms` and rust-i18n's flat
  keys do not give. `.ftl` files are plain text that Pontoon and Weblate edit.

To add a language: copy `locales/en.ftl` to `locales/<tag>.ftl`, translate the values, then add
the variant to `Lang` in `src/i18n.rs` (tag, native name, `include_str!`). The test
`every_language_has_every_english_message` fails until every message exists; a missing message
falls back to English at run time.

## What the screenshots show

`poc-shots/` at the repository root holds SVGs written by `--shots` from the same drawing code the terminal uses (the
buffer of ratatui's `TestBackend`, one positioned glyph per cell). Each follows the reader's light
or dark setting.

- `80x24/` and `120x36/`: `<lang>-<screen>.svg` for `language`, `server`, `access`, `review`,
  `review-long` (a long name, truncated), `install` (at 1 minute) and `install-done`.
- `themes/`: the install screen forced light and forced dark.
- `console/`: the fallback for `TERM=linux` or a non-UTF-8 locale.
- `tmux/`: text captures of the real binary in an 80x24 tmux pane, driven with `send-keys`,
  including CJK typed into a field and a live language switch.

## Findings

**CJK width.** `unicode-width` gives Han and kana two cells; ratatui lays out by it, so labels
padded by display width line up in all four languages (Server and Review tables). Truncation
counts cells and drops a wide character that would straddle the edge (`text::truncate`).

**Ambiguous-width glyphs.** Box drawing (`─ │ ┌`), `●`, `…`, `·`, `→` and the full block `█`
are East Asian Width "A". Most terminals draw them one cell wide, which is what ratatui assumes;
a terminal set to treat them as wide (an iTerm2 option, some Windows setups) shifts
every line that contains one. The screens avoid them inside text (`( )`, `(*)`, `>`, `-`, `...`,
coloured spaces for the bar) and test that list (`content_glyphs_are_not_ambiguous_width`), but
borders still use box drawing. No terminal with ambiguous-wide was tried.

**Wrapping.** ratatui wraps at spaces. A Chinese sentence has few of them, so a long run is one
"word" and a line can end early where Latin text such as `Kubernetes` sits in it (`tmux/switch-live.txt`,
80 columns). Wrapped lines also lose their hanging indent. It also does not apply line-start
rules, so a line can start with `。` or `、`. A UAX #14 line breaker (`unicode-linebreak`) would fix all three.

**Text input and IME.** Committed CJK text arrives as ordinary key events, in tmux and over SSH
alike: `tmux/zh-TW-access-typed.txt` types 陳怡君, moves left, deletes 怡 and inserts 小. The IME
composition (pre-edit) is drawn by the terminal emulator on the person's computer, at the
terminal's cursor; the app puts the real cursor after the text by display width, which is what
keeps the candidate window next to the field. Not tried by hand with a real IME in this PoC.

**Fallback.** With `TERM=linux` (the Linux virtual console: 512-glyph font, no CJK) or when
`LC_ALL`/`LC_CTYPE`/`LANG` is unset, `C`, `POSIX` or not UTF-8, the app stays in English, uses
ASCII borders, says why at the top, and shows the other languages as tags with "needs a UTF-8
terminal" (`console/`, `tmux/term-linux.txt`, `tmux/lang-C.txt`). Over SSH the locale arrives
only if the client sends it (`SendEnv LANG LC_*`) and the server accepts it (`AcceptEnv`).

**Plurals and numbers.** English needs `one`/`other` (`1 core`, `4 cores`; `1 thing to know`);
Chinese needs only `other`. Fluent's `NUMBER()` in Rust has no digit grouping, so
numbers are grouped in Rust (`1,210 MB`); all three languages group by thousands with a comma in
software UI. Durations are a Fluent message per language (`2 min 05 s`, `2 分 05 秒`).
Fluent wraps placeables in Unicode isolation marks by default; the terminal draws them as extra
cells, so `set_use_isolating(false)`.

**Colours.** Only the 16 ANSI colours plus bold, dim and reverse, so the terminal's palette
decides. Reverse-video slabs turned into dark bars on light themes and were removed; dim text is
readable in both themes in the SVGs, but how dim renders differs between terminals.

## Strings

zh-TW and zh-CN follow the zeabur-wording and zeabur-writing rules; one pass by another model
(Codex) suggested changes, and the ones that read better were taken. Log lines are tool output and stay English in every
language.
