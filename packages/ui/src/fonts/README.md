# Bundled fonts

Geist and Geist Mono, the UI and code faces of the Rastai design (`docs/design/ui-design.md` §4.2 and §4.5). `styles/base.css`
loads them with `@font-face` from this folder. Nothing is fetched at runtime, and no CDN is used.

| File | Used for |
|---|---|
| `Geist[wght].woff2` | `font_family_ui`, regular, medium and bold (variable, 100 to 900) |
| `Geist-Italic[wght].woff2` | italic text in the UI face |
| `GeistMono[wght].woff2` | `font_family_mono`: ids, tokens and every number (variable, 100 to 900) |
| `OFL.txt` | the SIL Open Font License 1.1 under which the files are distributed, shipped beside them |

## Source

- Project: https://github.com/vercel/geist-font
- Release: `v1.7.2` (tag `v1.7.2`, published 2026-06-01), asset `geist-font-v1.7.2.zip`
- Download: https://github.com/vercel/geist-font/releases/download/v1.7.2/geist-font-v1.7.2.zip
- Zip SHA-256: `7fc800d2ac6b92844895196e5041aca55d814c15db70c44f79b3b83ab82b04e2`
- Paths inside the zip: `geist-font/Geist/webfonts/Geist[wght].woff2`, `geist-font/Geist/webfonts/Geist-Italic[wght].woff2`,
  `geist-font/GeistMono/webfonts/GeistMono[wght].woff2`, `geist-font/OFL.txt`. The files are copied byte for byte; they are
  not subset or converted.

| File | SHA-256 |
|---|---|
| `Geist[wght].woff2` | `a369fcf5628ea2aa4e1b9e2ec6a5b3624e365bda588e1f0f2f12b564f728fbb8` |
| `Geist-Italic[wght].woff2` | `5aa7d48f114322a085337fad71e52664d6fd69331eb7e395af3b7d0025613da4` |
| `GeistMono[wght].woff2` | `fba8f577f38a2bbcbe818efa6348dd58f36303a10b8737c42fefad275be563ab` |
| `OFL.txt` | `c683bfbcc7e087f5d37a54ef628f10387c451a83ddc459b151403a164ac46c90` |

## License

Copyright 2024 The Geist Project Authors (https://github.com/vercel/geist-font). Licensed under the SIL Open Font License,
Version 1.1; the license text is in `OFL.txt`. Geist has no Reserved Font Name, so the files may be bundled unchanged.

## Updating

Take the new release's files, check them against the checksums of the release you take, and update this table and the
`@font-face` rules in `styles/base.css` in the same change. A new font name needs a contract entry (contracts 15.8).
