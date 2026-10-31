# Collection and privacy

[Back to the README](../README.md)

Owlytics uses no cookies and stores no raw IP addresses. The tracker skips
visitors sending Do Not Track (DNT), visitors sending Global Privacy Control
(GPC), and automated browsers. GPC collection is configurable; DNT and
automated-browser filtering stay active.

## Global Privacy Control

`OWLYTICS_RESPECT_GPC` controls whether Owlytics skips analytics from visitors
sending Global Privacy Control (GPC). It accepts `true` or `false` and defaults
to `true` when unset, including on existing installations. With `true`, both
the tracker (`navigator.globalPrivacyControl`) and the pageview and engagement
endpoints (`Sec-GPC: 1`) suppress collection. Set `OWLYTICS_RESPECT_GPC=false`
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
