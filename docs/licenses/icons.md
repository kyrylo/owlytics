# Report icons

Brand artwork identifies traffic sources and software. Names, logos and trademarks
remain the property of their respective owners; these assets imply no endorsement.
No Plausible assets or third-party icon services are used at runtime.

## Color artwork

The 38 files in `assets/brands/` retain their original colors and proportions.
`assets/brands/sources.json` records source URLs, immutable upstream revisions
where available, retrieval dates for vendor favicons, and SHA-256 checksums.
They are embedded in the executable and served from content-versioned local URLs.
This isolates SVG gradients and IDs and allows browsers to cache artwork across
reports and realtime refreshes. Icon colors do not change with row hover or theme.

- Chrome, Chromium, Firefox, Edge, Opera, Safari, Samsung Internet and Brave:
  [Browser Logos](https://github.com/alrra/browser-logos) revision
  `58881b84c4d73adc03c06fa2c275a7abee02d935`. Its collection code is under
  [MIT](Browser-Logos-MIT.txt); upstream explicitly reserves the logos and
  trademarks to their respective owners.
- Other color SVGs, except those below: [SVG Logos](https://github.com/gilbarbara/logos)
  revision `d6960edc1b7309d392c76e40d3a320df7f9ff85c`, with the collection's
  [CC0 notice](SVG-Logos-CC0.txt).
- Yahoo, Amazon, eBay, Instagram, Hacker News and Twitch: [Dashboard Icons](https://github.com/homarr-labs/dashboard-icons)
  revision `83f32e08257b66fbd971ac8d7645f5b38d52e85d`, with its
  [Apache 2.0 notice](Dashboard-Icons-Apache-2.0.txt).
- Google and Yandex: the respective vendors' public `favicon.ico` files.
- Copilot: Microsoft's public Copilot SVG favicon. The only modification is
  adding `viewBox="0 0 120 120"` to preserve scaling; paths and colors are unchanged.
  Both upstream and bundled checksums are recorded.

## Monochrome artwork

The 16 SVGs in `assets/icons/` retain the source geometry. All except Grok come
from [Simple Icons](https://github.com/simple-icons/simple-icons), under
[CC0 1.0](Simple-Icons-CC0.txt). `assets/icons/sources.json` records the source
URL, immutable revision and checksum. Grok comes from the SVG Logos revision
above under its collection's CC0 notice.

Most Simple Icons files use revision `90646e4bf7f3b89df6c37e65baf3e7bc759a517c`;
OpenAI uses release 11.15.0, revision `0af835e2949a9222965522780e5e0810d3c53382`.
The renderer uses the canonical brand colors from the corresponding Simple Icons
metadata. Inherently monochrome marks follow the dashboard's foreground color
in light and dark themes. CC0 does not grant trademark rights.
Amazon, Baidu, Ecosia, Yahoo, LinkedIn, Twitch, TikTok, Perplexity and Pinterest
have a neutral backplate so transparent lettering and dark artwork remain
recognizable on both dashboard backgrounds.

Channel/device symbols, UI controls and site monograms in `src/icons.rs` are
original Owlytics drawings under MIT. Website favicons are separately discovered
from tracked websites and cached in SQLite. A loaded favicon replaces the
monogram; transparent image pixels do not reveal it underneath.
