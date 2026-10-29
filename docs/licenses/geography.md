# Geographic display data

These embedded assets require no external requests or additional runtime service.

## Country outlines

`assets/country-shapes.tsv` derives from Natural Earth's 1:50m Admin 0 Countries:
https://github.com/nvkelso/natural-earth-vector/blob/master/geojson/ne_50m_admin_0_countries.geojson

Natural Earth data is public domain. Terms:
https://www.naturalearthdata.com/about/terms-of-use/

The generator applies a Mercator projection in a 475 × 335 frame (scale 75,
translation 237.5, 223⅓), simplifies the outlines, merges map units sharing an
ISO country code, omits Antarctica and marks very small countries with circles.
The result contains 236 country shapes. The country list includes places that
cannot be represented on the map. Projection runs only when rebuilding the asset;
the browser loads a local, versioned SVG sprite without a mapping library.

## City names

`assets/city-names.tsv` derives from GeoNames, copyright GeoNames contributors,
under [Creative Commons Attribution 4.0 International](https://creativecommons.org/licenses/by/4.0/).

Sources: https://download.geonames.org/export/dump/cities500.zip and
https://download.geonames.org/export/dump/admin1CodesASCII.txt.
Data description and license: https://download.geonames.org/export/dump/readme.txt.

The generator keeps the GeoNames ID, country code, city and first-level
administrative name, sorts by ID, and joins the two names for display. Coordinates,
population and other fields are discarded. The asset contains 236,764 names.
`cities500` includes cities over 500 people and administrative seats; it does not
cover every settlement. Unknown or mismatched IDs keep their original values.
Traffic counts and stored location keys are unchanged. GeoNames does not endorse
Owlytics. The dashboard and downloaded reports credit GeoNames when names are used.

The app reads the sorted assets directly with bounded binary searches. It does
not allocate a lookup table containing every city.

## Location identities

`assets/region-keys.tsv` and `assets/city-keys.tsv` derive from the same pinned
CLDR and GeoNames inputs. They contain 16,515 region aliases and 508,383 city
aliases. Names and ASCII names are scoped to their country and administrative
division. Region names resolve only when they identify one CLDR subdivision;
GeoNames's `State of …` form may use that exact subdivision's shorter name.
Metro Manila and National Capital Region are linked explicitly by
[GeoNames's administrative entry](https://www.geonames.org/7521311/national-capital-region.html).
Five ambiguous region aliases and 10,791 ambiguous city aliases are omitted.
Unknown places retain their original keys. No country-wide city-name fallback
or approximate spelling match is used.

The keys join DB-IP names to the same region codes and city IDs used by imported
reports. They do not change the underlying geolocation provider's accuracy or
extend the settlement coverage of `cities500`. Country/city boundaries remain
part of each lookup. Coordinate, population and personal data are not included.
The GeoNames and Unicode licenses above also apply to these derived assets.

## Rebuilding

Download the three files into a directory, naming the Natural Earth file
`countries.geojson`, then run `python3 bin/build-geography.py SOURCE_DIRECTORY`.
This optional maintenance command is not part of deployment or startup.
Then run `python3 bin/build-location-keys.py SOURCE_DIRECTORY` to rebuild the
identity aliases from those same sources and `assets/region-names.tsv`.

Source snapshots retrieved 2026-10-27 (SHA-256):

| File | SHA-256 |
| --- | --- |
| countries.geojson | `3e458fc036ad0a66411f2c1e6cac49c5d7bfb81cb1123bc513b22511a2b7fdeb` |
| cities500.zip | `4c16306582581fa599d246ef103d24d51ed7cdd615603e6bbf393bcd6b3eff42` |
| admin1CodesASCII.txt | `2e4867f5397acb056ea26377601b57af37cb4c81399770225018e9e439e91ad3` |

Country and ISO subdivision labels separately use Unicode CLDR under
Unicode-3.0; see [its notice](Unicode-3.0.txt).
