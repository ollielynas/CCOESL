# Shell fonts

The Brutal style's type (issue #42), compiled into the shell with `include_bytes!` in
`src/theme.rs`. Apps never carry fonts, so these don't count against the app size budget.

| File | Font | Source | Licence |
|---|---|---|---|
| `SpaceGrotesk-Regular.ttf` | Space Grotesk 400 | [floriankarsten/space-grotesk](https://github.com/floriankarsten/space-grotesk) `fonts/ttf/static/` | SIL OFL 1.1, `OFL-SpaceGrotesk.txt` |
| `SpaceGrotesk-Bold.ttf` | Space Grotesk 700 | same | same |
| `SpaceMono-Regular.ttf` | Space Mono 400 | [google/fonts](https://github.com/google/fonts) `ofl/spacemono/` | SIL OFL 1.1, `OFL-SpaceMono.txt` |

Each is a Latin subset (Google Fonts' `latin` range), unhinted, which takes the three from
about 330 KB to 69 KB. Neither licence declares a Reserved Font Name, so the subsets keep their
names. Anything outside the subset (`›`, `…`, emoji, icons) falls through to egui's default fonts
and Phosphor, which `theme::font_definitions` keeps after these in every family.

To regenerate, with `pip install fonttools`:

```sh
pyftsubset <Font>.ttf --no-hinting --desubroutinize --layout-features='kern,liga' \
  --unicodes='U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0304,U+0308,U+0329,U+2000-206F,U+20AC,U+2122,U+2191,U+2193,U+2212,U+2215,U+FEFF,U+FFFD' \
  --output-file=<Font>.ttf
```
