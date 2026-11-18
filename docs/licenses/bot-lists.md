# Bot-filter data

The bundled lists in `assets/bot-lists/` derive from these independent sources:

| Asset | Source | License | Transformation |
| --- | --- | --- | --- |
| `crawlers.txt` | [crawler-user-agents](https://github.com/monperrus/crawler-user-agents), Martin Monperrus and contributors | [MIT](Crawler-User-Agents-MIT.txt) | Sorted, deduplicated regular-expression patterns; metadata and example strings omitted. |
| `referrers.txt` | [Matomo referrer-spam-list](https://github.com/matomo-org/referrer-spam-list), Matomo and contributors | [CC0 1.0](Bot-Lists-CC0.txt), declared in upstream `composer.json` | Lowercase IDNA hostnames, sorted and deduplicated. |
| `hosting.txt` | [ipverse/as-ip-blocks](https://github.com/ipverse/as-ip-blocks), ipverse contributors | [CC0 1.0](Bot-Lists-CC0.txt) | Selected hosting ASNs, aggregated into IPv4 and IPv6 CIDRs. |

Owlytics's provider selection is maintained in `bin/update-bot-lists.py`.
It does not use a blanket VPN blocklist or all networks belonging to companies
that also operate consumer ISPs. Hosting networks can nevertheless carry real
visitors through VPNs, proxies, or relays. List providers do not endorse Owlytics.

Exact revisions, source URLs, SHA-256 checksums, and counts are recorded in
[sources.json](../../assets/bot-lists/sources.json). No upstream source code is
executed by the updater. Source downloads occur only during an explicit
maintainer update; the application needs no network access for these checks.
