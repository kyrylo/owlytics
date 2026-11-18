# Bot and spam filtering

[Back to the README](../README.md)

Owlytics filters new traffic automatically before it reaches SQLite. Pageviews,
engagement updates, and outbound clicks share the collection filters. Filtered
requests return `204 No Content`, so trackers do not retry them. The response
header `X-Owlytics-Dropped` explains a server-side drop: `user-agent`,
`datacenter`, `referrer-spam`, or `rate-limit`. These filters do not delete
previously collected or imported traffic.

## What is filtered

- **Browser automation.** The tracker skips browsers exposing
  `navigator.webdriver`. Crawlers that do not run JavaScript do not trigger it.
- **Known crawlers and automated clients.** The server checks a bundled snapshot
  of crawler-user-agents plus signatures for headless browsers and HTTP clients.
  Missing, blank, and oversized User-Agent headers are also skipped.
- **Referrer spam.** Pageviews from domains in Matomo's spam list are skipped,
  including subdomains. Matching uses the parsed hostname, not text in paths or
  queries. People clicking links from AI assistants are still counted.
- **Hosting networks.** IPv4 and IPv6 ranges from selected hosting ASNs are
  filtered, including IPv4-mapped IPv6 connections. The initial selection covers
  AWS, Google Cloud, DigitalOcean, Hetzner, OVH, Linode, Vultr, Alibaba Cloud,
  Tencent Cloud, and Scaleway. This is not a complete list of all data centers.
- **Bursts.** Within each 60-second window, one site/IP/User-Agent combination
  can send up to 120 distinct pageviews and 600 total collector requests.
  Repeated pageview IDs do not consume extra pageview slots. Exceeding either
  limit drops subsequent pageviews, engagement, and clicks for the remainder
  of that window. Earlier accepted events remain in the reports.

Site-origin validation, DNT, and the configured GPC policy still apply.
Engagement and clicks require an existing pageview; they cannot create visits
when their parent was filtered.

## Real visitors on hosting networks

A hosting IP is a useful signal, not proof of automation. VPNs, corporate
proxies, and privacy relays can share these ranges with bots. A busy shared
connection whose browsers use the same User-Agent can also hit the burst limit.

`OWLYTICS_FILTER_DATACENTERS=true` is the default. Set it to `false` to disable
only hosting-network filtering, or use `OWLYTICS_BOT_ALLOWED_IPS` for narrow
exceptions, for example `203.0.113.7/32,2001:db8:1234::/48`. Exceptions bypass
only the IP list; crawler, spam, and burst filters still apply. Restart or
redeploy after changing these settings.

Behind a reverse proxy, configure `OWLYTICS_TRUSTED_PROXIES` with its actual
addresses or networks. Owlytics walks `X-Forwarded-For` from the trusted peer
toward the visitor, stopping at the first untrusted hop. Headers supplied
directly by an untrusted client cannot override its IP. A misconfigured proxy
can make all visits appear to come from the proxy's hosting address.

## Privacy and maintenance

Checks run locally. No requests or visitor data are sent to list providers.
Raw IP addresses and full User-Agent strings are not retained. Burst counters
use a site-scoped HMAC with an in-memory key; the counters and key are reset on
the first request after each 60-second window and on restart. Inactive state
can remain in process memory until the next request, but is never written to
SQLite or logs. At 20,000 identities per window, new identities bypass only the
burst check to keep memory bounded; existing counters and static checks remain.

Lists are bundled into the executable. Updating Owlytics picks up the snapshot
in that release; running instances do not download new rules automatically.
The exact upstream commits, source checksums, provider metadata, output checksums,
and counts are in [sources.json](../assets/bot-lists/sources.json).
See [attribution and licenses](licenses/bot-lists.md).

For maintainers, run `python3 bin/update-bot-lists.py --refresh` to prepare a new
snapshot. Review changes, particularly hosting-network ownership and broad
prefix additions, run `bin/test`, then build and release normally. Running the
same command without `--refresh` reproduces the pinned snapshot and verifies
source checksums. Downloads and validation finish before files are replaced;
a network or validation failure leaves the existing snapshot in place.

## Limits

This does not reproduce Plausible Cloud's private traffic-pattern algorithm or
its continuously maintained data-center list. Plausible describes its layers
in [its bot-filtering documentation](https://plausible.io/docs/bot-traffic-filtering).
Owlytics's behavioral check is a burst limit, not a classifier. Slow automation,
residential proxies, rotating User-Agents, and spoofed browser signals can still
get through. No measured detection rate or feature parity with Plausible Cloud
is claimed. Neither low engagement nor a visitor's country is enough to drop a
visit on its own.
