# Report icons

The 45 SVGs in `assets/icons/` come from [Simple Icons](https://github.com/simple-icons/simple-icons),
under [CC0 1.0](Simple-Icons-CC0.txt). `assets/icons/sources.json` records each
unmodified file's source URL, immutable revision and SHA-256 checksum.

Most use revision `90646e4bf7f3b89df6c37e65baf3e7bc759a517c`. Bing, OpenAI,
LinkedIn, Slack, Teams, Amazon and Edge use release 11.15.0, revision
`0af835e2949a9222965522780e5e0810d3c53382`. Their names and marks identify
traffic sources/software; CC0 does not grant trademark rights or imply endorsement.

The renderer sets the bundled paths to the current text color and hides decorative
SVGs from assistive technology. The remaining inline symbols, channel/device icons
and site monograms in `src/icons.rs` are original Owlytics drawings under MIT.
No Plausible assets are included. These report icons are embedded in the executable
and do not contact an icon service. Website-card favicons are separately fetched
from the tracked sites and cached in SQLite.
