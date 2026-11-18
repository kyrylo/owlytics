# Collection and privacy

[Back to the README](../README.md)

Owlytics uses no cookies and stores no raw IP addresses. The tracker skips
visitors sending Do Not Track (DNT), visitors sending Global Privacy Control
(GPC), and automated browsers. GPC collection is configurable; DNT and
automated-browser filtering stay active.

Server-side [bot and spam filtering](bot-filtering.md) also excludes known
crawlers, referrer spam, selected hosting networks, and excessive request bursts.
Checks run locally; short-lived burst counters use in-memory hashes without
storing raw IP addresses or User-Agent strings. Hosting-network filtering can
exclude some real VPN or proxy users and supports explicit exceptions.

## Global Privacy Control

`OWLYTICS_RESPECT_GPC` controls whether Owlytics skips analytics from visitors
sending Global Privacy Control (GPC). It accepts `true` or `false` and defaults
to `true` when unset, including on existing installations. With `true`, both
the tracker (`navigator.globalPrivacyControl`) and the pageview, engagement, and
outbound-click endpoints (`Sec-GPC: 1`) suppress collection. Set `OWLYTICS_RESPECT_GPC=false`
in the server environment to include otherwise eligible GPC traffic. DNT and
automated-browser filtering remain active in either mode.

Blanket GPC suppression is Owlytics's collection policy; GPC does not universally
prohibit first-party analytics. Suppression excludes those visitors' pageviews
and engagement, reducing counts compared with collecting that traffic. Disabling
it can increase visitor, visit, and pageview counts and change engagement metrics.
Previously skipped activity is not recovered, and existing data is unchanged.

Restart Owlytics after changing the option (redeploy with Kamal, or recreate a
Docker container with `-e OWLYTICS_RESPECT_GPC=false`). The server applies the
setting at startup and embeds it in `/script.js`. The tracker uses
`Cache-Control: public, no-cache` and a content-based ETag, so browsers revalidate
it and receive the new setting on the next full page load. Already open pages
keep their loaded tracker until reloaded; direct requests use the server's current
setting. Any proxy serving the tracker must honor its revalidation headers.

## Outbound links

The standard tracker automatically records HTTP(S) link clicks to other domains,
including new-tab and middle-click navigation. Links to the same hostname or its
`www` equivalent are internal; other subdomains count as outbound. `mailto:`,
`tel:`, and other non-HTTP links are excluded. No snippet change is needed after
an upgrade; the new tracker takes effect on the next full page load.

Destination URLs contain only the origin and path. Credentials, query strings,
and fragments are removed in both the browser and the collector. Source paths
and destination paths can still contain information placed in URLs by the site.
Clicks inherit the originating pageview's anonymous visitor and audience
attributes, without creating cookies or another visitor identifier. DNT, GPC,
and bot filtering apply to clicks as well.

The **Goals** report includes **Outbound Link: Click**, with **Uniques**, **Total**,
and **CR** (conversion rate). Selecting the goal filters the chart and audience
reports to conversions and shows the `url` property breakdown. Selecting a URL
narrows that view further; Top pages shows the pages where clicks occurred.
Unique conversions count distinct anonymous visitors across all matching clicks,
including clicks from different pages. Conversion rate divides those visitors by
the collected visitor audience for the same period and audience filters, without
restricting that denominator to the selected goal or URL. Visitors whose pageview
preceded the period but clicked within it are included in that audience.
Historical traffic imports do not contain outbound conversions; conversion rates
use collected data only. Clicks remain separate from pageview, visit, bounce, and
duration metrics.

Dashboard exports include `conversions.csv` and up to 300 URL rows in
`custom_props.csv`, sorted by unique conversions.
