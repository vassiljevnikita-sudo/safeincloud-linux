# Bundled fonts

These font files are distributed under the SIL Open Font License 1.1, with the
complete upstream notices included alongside them. Original font binaries from
the Google Fonts repository were retained without altering their font data;
the variable-font filenames omit the `[wght]` suffix for simpler packaging.

- Atkinson Hyperlegible Next: https://github.com/google/fonts/tree/main/ofl/atkinsonhyperlegiblenext
  — `AtkinsonHyperlegibleNext[wght].ttf`, `OFL.txt`.
- Spectral: https://github.com/google/fonts/tree/main/ofl/spectral
  — `Spectral-Light.ttf`, `Spectral-Regular.ttf`, `OFL.txt`.
- Red Hat Mono: https://github.com/google/fonts/tree/main/ofl/redhatmono
  — `RedHatMono[wght].ttf`, `OFL.txt`.

The Rust application embeds these files and registers a local cache copy with
Fontconfig for its own GTK/Pango process. It does not change global desktop font
settings or contact a font service at runtime.
