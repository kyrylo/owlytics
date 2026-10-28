#!/usr/bin/env python3
"""Build local display assets from Natural Earth and GeoNames downloads.

Usage: python3 bin/build-geography.py SOURCE_DIRECTORY
Inputs and attribution are documented in docs/licenses/geography.md.
The application needs neither Python nor the source downloads at runtime.
"""
import hashlib
import json
import math
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = Path(sys.argv[1])


def simplify(points, tolerance=0.45):
    """Retain a polygon's outline using point-to-segment distance in SVG units."""
    if len(points) <= 2:
        return points
    a, b = points[0], points[-1]
    dx, dy = b[0] - a[0], b[1] - a[1]
    length = dx * dx + dy * dy
    best, index = 0, 0
    for i, (x, y) in enumerate(points[1:-1], 1):
        t = max(0, min(1, ((x-a[0])*dx + (y-a[1])*dy)/length)) if length else 0
        distance = (x-a[0]-t*dx)**2 + (y-a[1]-t*dy)**2
        if distance > best:
            best, index = distance, i
    if best <= tolerance**2:
        return [a, b]
    return simplify(points[:index+1], tolerance)[:-1] + simplify(points[index:], tolerance)


def number(value):
    return f"{value:.2f}".rstrip("0").rstrip(".")


shapes = {}
features = json.loads((SOURCE / "countries.geojson").read_text())["features"]
for feature in features:
    code = feature["properties"]["ISO_A2_EH"]
    if len(code) != 2 or not code.isascii() or not code.isupper() or code == "AQ":
        continue
    geometry = feature["geometry"]
    polygons = geometry["coordinates"] if geometry["type"] == "MultiPolygon" else [geometry["coordinates"]]
    paths, all_points = [], []
    for polygon in polygons:
        for ring in polygon:
            points = [(2*(lon+180), 2*(85-lat)) for lon, lat in ring]
            assert all(math.isfinite(x) and math.isfinite(y) for x,y in points)
            # Natural Earth splits shapes at the antimeridian; never join
            # a polygon across the entire map by accident.
            assert all(abs(a[0]-b[0]) < 360 for a,b in zip(points, points[1:])), code
            all_points.extend(points)
            points = simplify(points)
            if len(points) >= 4:
                paths.append("M" + "L".join(f"{number(x)},{number(y)}" for x,y in points[:-1]) + "Z")
    # Small sovereign states and islands remain reachable, even when their
    # true outlines are below a pixel at dashboard size.
    xs, ys = zip(*all_points)
    small = max(max(xs)-min(xs), max(ys)-min(ys)) < 3
    if small or not paths:
        x, y = (min(xs)+max(xs))/2, (min(ys)+max(ys))/2
        r = 1.8
        path = f"M{number(x-r)},{number(y)}a{r},{r} 0 1,0 {r*2},0a{r},{r} 0 1,0 -{r*2},0Z"
    else:
        path = "".join(paths)
    # Several map units (for example Australian island territories) share
    # an ISO code and belong to the same clickable country.
    shapes[code] = shapes.get(code, "") + path
(ROOT / "assets/country-shapes.tsv").write_text("".join(f"{code}\t{path}\n" for code,path in sorted(shapes.items())))

regions = {}
for line in (SOURCE / "admin1CodesASCII.txt").read_text().splitlines():
    code, name, *_ = line.split("\t")
    regions[code] = name
cities = {}
with zipfile.ZipFile(SOURCE / "cities500.zip") as archive:
    with archive.open("cities500.txt") as stream:
        for line in stream:
            values = line.decode("utf-8").rstrip("\n").split("\t")
            identity, name, country = int(values[0]), values[1], values[8]
            region = regions.get(f"{country}.{values[10]}", "")
            label = f"{name}, {region}" if region and region != name else name
            assert len(country) == 2 and country.isascii() and country.isupper()
            assert not any(ord(c) < 32 for c in label)
            assert 0 < identity < 2**32 and identity not in cities
            cities[identity] = (country, label)
(ROOT / "assets/city-names.tsv").write_text("".join(f"{identity}\t{country}\t{label}\n" for identity,(country,label) in sorted(cities.items())))
for name in ["countries.geojson", "cities500.zip", "admin1CodesASCII.txt"]:
    print(f"{name}: {hashlib.sha256((SOURCE / name).read_bytes()).hexdigest()}")
print(f"Generated {len(shapes)} country shapes and {len(cities)} city labels.")
