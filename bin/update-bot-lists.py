#!/usr/bin/env python3
"""Refresh bundled bot lists, or reproduce them from sources.json (default).

Run with --refresh to resolve new upstream commits. Review the resulting diff,
then run bin/test before releasing. No downloads happen in the running server.
"""
import argparse
import hashlib
import ipaddress
import json
import re
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / "assets/bot-lists"
REPOS = {
    "crawlers": ("monperrus/crawler-user-agents", "master"),
    "referrers": ("matomo-org/referrer-spam-list", "master"),
    "hosting": ("ipverse/as-ip-blocks", "master"),
}
# Deliberately selected hosting networks, not a blanket VPN or ISP blocklist.
# Review ownership when refreshing. Do not include Google's general AS15169.
HOSTING = {
    16509: "Amazon AWS", 14618: "Amazon AWS", 396982: "Google Cloud",
    14061: "DigitalOcean", 24940: "Hetzner", 16276: "OVH",
    63949: "Linode", 20473: "Vultr", 45102: "Alibaba Cloud",
    132203: "Tencent Cloud", 45090: "Tencent Cloud", 12876: "Scaleway",
}


def download(url):
    request = urllib.request.Request(url, headers={"User-Agent": "Owlytics-list-updater"})
    with urllib.request.urlopen(request, timeout=30) as response:
        data = response.read(5_000_001)
    if len(data) > 5_000_000:
        raise ValueError(f"Oversized source: {url}")
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--refresh", action="store_true")
    args = parser.parse_args()
    old = json.loads((DEST / "sources.json").read_text())
    revisions = old["revisions"].copy()
    if args.refresh:
        for key, (repo, branch) in REPOS.items():
            revisions[key] = json.loads(download(
                f"https://api.github.com/repos/{repo}/commits/{branch}"
            ))["sha"]
    assert all(re.fullmatch(r"[0-9a-f]{40}", rev) for rev in revisions.values())
    sources = []

    def fetch(key, filename):
        repo, _ = REPOS[key]
        url = f"https://raw.githubusercontent.com/{repo}/{revisions[key]}/{filename}"
        data = download(url)
        sources.append({"url": url, "sha256": hashlib.sha256(data).hexdigest()})
        return data

    crawlers = json.loads(fetch("crawlers", "crawler-user-agents.json"))
    patterns = sorted(set(row["pattern"] for row in crawlers))
    assert 500 < len(patterns) < 10_000
    assert all(p and "\n" not in p and "\r" not in p for p in patterns)
    domains = set()
    for line in fetch("referrers", "spammers.txt").decode().splitlines():
        if not line.strip():
            continue
        domain = line.strip().rstrip(".").encode("idna").decode().lower()
        assert "." in domain and len(domain) <= 253
        assert all(re.fullmatch(r"[a-z0-9](?:[a-z0-9-]*[a-z0-9])?", label)
                   for label in domain.split(".")), domain
        domains.add(domain)
    assert 500 < len(domains) < 100_000
    networks = {4: [], 6: []}
    providers = []
    for asn, name in HOSTING.items():
        entry = json.loads(fetch("hosting", f"as/{asn}/aggregated.json"))
        assert entry["asn"] == asn
        assert entry["metadata"]["category"] == "hosting", f"Review changed ownership of AS{asn}"
        providers.append({"asn": asn, "provider": name, "metadata": entry["metadata"]})
        assert entry["prefixes"]["ipv4"]
        for family in [4, 6]:
            for prefix in entry["prefixes"][f"ipv{family}"]:
                net = ipaddress.ip_network(prefix)
                assert net.version == family and net.is_global and net.prefixlen >= (8 if family == 4 else 19), prefix
                networks[family].append(net)
    cidrs = [str(net) for family in [4, 6]
             for net in ipaddress.collapse_addresses(networks[family])]
    assert 500 < len(cidrs) < 100_000
    files = {
        DEST / "crawlers.txt": "\n".join(patterns) + "\n",
        DEST / "referrers.txt": "\n".join(sorted(domains)) + "\n",
        DEST / "hosting.txt": "\n".join(cidrs) + "\n",
        ROOT / "docs/licenses/Crawler-User-Agents-MIT.txt": fetch("crawlers", "LICENSE").decode(),
        ROOT / "docs/licenses/Bot-Lists-CC0.txt": fetch("hosting", "LICENSE").decode(),
    }
    # Record Matomo's license declaration alongside the exact data sources.
    assert json.loads(fetch("referrers", "composer.json"))["license"] == "CC0-1.0"
    manifest = {"revisions": revisions, "sources": sources, "hosting_providers": providers,
                "counts": {"crawler_patterns": len(patterns), "referrer_domains": len(domains),
                           "hosting_ranges": len(cidrs)},
                "outputs": {str(path.relative_to(ROOT)): hashlib.sha256(data.encode()).hexdigest()
                            for path, data in files.items()}}
    if not args.refresh:
        # A pinned source must never silently change under the same revision.
        assert not old.get("sources") or old["sources"] == sources, "Source checksum mismatch"
    # All downloads and validation finish before replacing any checked-in file.
    files[DEST / "sources.json"] = json.dumps(manifest, indent=2) + "\n"
    for path, data in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_suffix(path.suffix + ".tmp")
        temporary.write_text(data)
        temporary.replace(path)
    print(json.dumps(manifest["counts"]))


if __name__ == "__main__":
    main()
