#!/usr/bin/env python3
"""Build unambiguous local location identities from the pinned geography sources.

Usage: python3 bin/build-location-keys.py SOURCE_DIRECTORY
No source download, Python process or in-memory city index is needed at runtime.
"""
import collections
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = Path(sys.argv[1])
regions = collections.defaultdict(set)


def add(table, key, identity):
    assert not any(c in key or c in identity for c in "\t\r\n")
    table[key.lower()].add(identity)


def unique(table, key):
    values = table.get(key.lower(), set())
    return next(iter(values)) if len(values) == 1 else None


for line in (ROOT / "assets/region-names.tsv").read_text().splitlines():
    code, name = line.split("\t")
    country = code[:2].upper()
    identity = f"{country}: {country}-{code[2:].upper()}"
    for alias in [code, f"{country}-{code[2:]}", name]:
        add(regions, f"{country}: {alias}", identity)

# GeoNames explicitly lists Metro Manila as the same administrative division:
# https://www.geonames.org/7521311/national-capital-region.html
add(regions, "PH: National Capital Region", "PH: PH-00")

admins = {}
for line in (SOURCE / "admin1CodesASCII.txt").read_text().splitlines():
    code, name, ascii_name, _ = line.split("\t")
    country = code[:2]
    # GeoNames uses, for example, "State of Berlin" where CLDR uses "Berlin".
    # Resolve only an exact, unique subdivision name within the same country.
    candidates = {identity for alias in [name, ascii_name, name.removeprefix("State of ")]
                  if (identity := unique(regions, f"{country}: {alias}"))}
    identity = next(iter(candidates)) if len(candidates) == 1 else None
    if identity:
        for alias in [name, ascii_name]:
            add(regions, f"{country}: {alias}", identity)
    admins[code] = (name, ascii_name)

cities = collections.defaultdict(set)
with zipfile.ZipFile(SOURCE / "cities500.zip") as archive:
    with archive.open("cities500.txt") as stream:
        for line in stream:
            values = line.decode("utf-8").rstrip("\n").split("\t")
            number, country = values[0], values[8]
            names = {values[1], values[2]} - {""}
            states = set(admins.get(f"{country}.{values[10]}", ("", "")))
            identity = f"{country}: {number}"
            for name in names:
                for state in states:
                    # Preserve both the display label and the older DB-IP format.
                    add(cities, f"{country}: {name}, {state}", identity)
                    if not state or state == name:
                        add(cities, f"{country}: {name}", identity)
                    canonical = unique(regions, f"{country}: {state}")
                    if canonical:
                        add(cities, f"{country}: {name}, {canonical.split(': ', 1)[1]}", identity)

for name, table in [("region", regions), ("city", cities)]:
    rows = [(key, next(iter(values))) for key, values in table.items() if len(values) == 1]
    rows.sort(key=lambda row: row[0].encode("utf-8"))
    (ROOT / f"assets/{name}-keys.tsv").write_text("".join(f"{key}\t{value}\n" for key, value in rows))
    print(f"{name}: {len(rows)} aliases; {len(table)-len(rows)} ambiguous aliases omitted")
