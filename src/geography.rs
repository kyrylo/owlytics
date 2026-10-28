//! Local geographic display data. See docs/licenses/geography.md and Unicode-3.0.txt.

pub fn country_shapes() -> impl Iterator<Item = (&'static str, &'static str)> {
    include_str!("../assets/country-shapes.tsv")
        .lines()
        .filter_map(|line| line.split_once('\t'))
}

// Binary-search the sorted UTF-8 asset without loading a city database or
// allocating an index. Every byte offset is aligned to a complete TSV line.
fn city_name(country: &str, identity: u32) -> Option<&'static str> {
    let data = include_str!("../assets/city-names.tsv");
    let (mut low, mut high) = (0, data.len());
    while low < high {
        let middle = low + (high - low) / 2;
        let start = data.as_bytes()[..middle]
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |i| i + 1);
        let end = start + data[start..].find('\n').unwrap_or(data.len() - start);
        let mut fields = data[start..end].splitn(3, '\t');
        let candidate = fields.next()?.parse::<u32>().ok()?;
        match candidate.cmp(&identity) {
            std::cmp::Ordering::Less => low = end + 1,
            std::cmp::Ordering::Greater => high = start,
            std::cmp::Ordering::Equal => {
                let code = fields.next()?;
                return (country.is_empty() || code == country)
                    .then(|| fields.next())
                    .flatten();
            }
        }
    }
    None
}

pub fn imported_city(value: &str) -> Option<&'static str> {
    let (country, identity) = value.split_once(": ").unwrap_or(("", value));
    city_name(country, identity.parse().ok()?)
}

// Keys are sorted by UTF-8 bytes. Searching the embedded file avoids a second
// database or an allocation proportional to the number of geographic places.
fn identity(data: &'static str, key: &str) -> Option<&'static str> {
    let (mut low, mut high) = (0, data.len());
    while low < high {
        let middle = low + (high - low) / 2;
        let start = data.as_bytes()[..middle]
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |i| i + 1);
        let end = start + data[start..].find('\n').unwrap_or(data.len() - start);
        let (candidate, value) = data[start..end].split_once('\t')?;
        match candidate.cmp(key) {
            std::cmp::Ordering::Less => low = end + 1,
            std::cmp::Ordering::Greater => high = start,
            std::cmp::Ordering::Equal => return Some(value),
        }
    }
    None
}

pub(crate) fn location_key(dimension: &str, value: &str) -> String {
    let data = match dimension {
        "region" => include_str!("../assets/region-keys.tsv"),
        "city" => include_str!("../assets/city-keys.tsv"),
        _ => return value.to_owned(),
    };
    if let Some(found) = identity(data, &value.to_lowercase()) {
        return found.to_owned();
    }
    if dimension == "city"
        && let Some((country, place)) = value.split_once(": ")
        && let Some((name, region)) = place.rsplit_once(", ")
    {
        let canonical = location_key("region", &format!("{country}: {region}"));
        if let Some((_, region)) = canonical.split_once(": ")
            && let Some(found) =
                identity(data, &format!("{country}: {name}, {region}").to_lowercase())
        {
            return found.to_owned();
        }
    }
    // Ambiguous, unknown and mismatched-country places must retain their keys.
    value.to_owned()
}
pub fn country_name(code: &str) -> &str {
    include_str!("../assets/country-names.tsv")
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(key, _)| *key == code)
        .map(|(_, name)| name)
        .unwrap_or(code)
}
pub fn region_name(code: &str) -> &str {
    let key = code.replace('-', "").to_ascii_lowercase();
    include_str!("../assets/region-names.tsv")
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(id, _)| *id == key)
        .map(|(_, name)| name)
        .unwrap_or(code)
}
pub fn label(dimension: &str, value: &str) -> String {
    match dimension {
        "country" => country_name(value).to_owned(),
        "region" => region_name(
            value
                .split_once(": ")
                .map(|(_, name)| name)
                .unwrap_or(value),
        )
        .to_owned(),
        "city" => imported_city(value)
            .unwrap_or_else(|| {
                value
                    .split_once(": ")
                    .map(|(_, name)| name)
                    .unwrap_or(value)
            })
            .to_owned(),
        _ => value.to_owned(),
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn native_names_and_imported_identifiers_share_unambiguous_keys() {
        for alias in ["US: California", "US: US-CA", "US: usca"] {
            assert_eq!(super::location_key("region", alias), "US: US-CA");
        }
        assert_eq!(
            super::location_key("region", "DE: State of Berlin"),
            "DE: DE-BE"
        );
        for alias in [
            "US: San Francisco, California",
            "US: San Francisco, US-CA",
            "US: 5391959",
        ] {
            assert_eq!(super::location_key("city", alias), "US: 5391959");
        }
        assert_eq!(
            super::location_key("city", "DE: Berlin, Berlin"),
            "DE: 2950159"
        );
        assert_eq!(
            super::location_key("city", "US: Springfield, Illinois"),
            "US: 4250542"
        );
        assert_eq!(
            super::location_key("city", "US: Springfield, Massachusetts"),
            "US: 4951788"
        );
    }

    #[test]
    fn location_identity_does_not_guess_across_countries_or_ambiguous_names() {
        for value in [
            "FR: California",
            "FR: US-CA",
            "US: <unknown>",
            "",
            "US: 999999999",
        ] {
            for dimension in ["region", "city"] {
                assert_eq!(super::location_key(dimension, value), value);
            }
        }
        for value in [
            "US: Springfield",
            "US: San Francisco, Unknown Region",
            "FR: San Francisco, California",
        ] {
            assert_eq!(super::location_key("city", value), value);
        }
        for data in [
            include_str!("../assets/region-keys.tsv"),
            include_str!("../assets/city-keys.tsv"),
        ] {
            let mut previous = "";
            for (i, line) in data.lines().enumerate() {
                let (key, value) = line.split_once('\t').unwrap();
                assert!(key > previous, "Duplicate or unsorted alias: {key}");
                if i % 997 == 0 {
                    assert_eq!(super::identity(data, key), Some(value));
                }
                previous = key;
            }
            let (last, value) = data.lines().last().unwrap().split_once('\t').unwrap();
            assert_eq!(super::identity(data, last), Some(value));
        }
    }
    #[test]
    fn country_and_imported_region_codes_have_readable_labels() {
        assert_eq!(super::country_name("US"), "United States");
        assert_eq!(super::country_name("PH"), "Philippines");
        assert_eq!(super::label("region", "US: US-CA"), "California");
        assert_eq!(super::label("region", "GB: GB-ENG"), "England");
        assert_eq!(super::label("region", "PH: Metro Manila"), "Metro Manila");
        assert_eq!(super::label("region", "XX: custom"), "custom");
    }

    #[test]
    fn geonames_labels_are_local_searchable_and_preserve_unknown_values() {
        assert_eq!(
            super::label("city", "US: 5391959"),
            "San Francisco, California"
        );
        assert_eq!(
            super::label("city", "DE: 2950159"),
            "Berlin, State of Berlin"
        );
        assert_eq!(
            super::label("city", "US: San Francisco, California"),
            "San Francisco, California"
        );
        assert_eq!(super::label("city", "FR: 5391959"), "5391959");
        assert_eq!(super::label("city", "US: 4294967295"), "4294967295");
        assert_eq!(super::label("city", "US: <unknown>"), "<unknown>");
        let data = include_str!("../assets/city-names.tsv");
        let mut previous = 0;
        for (index, line) in data.lines().enumerate() {
            let mut fields = line.splitn(3, '\t');
            let id = fields.next().unwrap().parse::<u32>().unwrap();
            let country = fields.next().unwrap();
            let label = fields.next().unwrap();
            assert!(id > previous);
            previous = id;
            if index % 997 == 0 {
                assert_eq!(super::city_name(country, id), Some(label));
            }
        }
        for line in [data.lines().next().unwrap(), data.lines().last().unwrap()] {
            let mut fields = line.splitn(3, '\t');
            let id = fields.next().unwrap().parse().unwrap();
            let country = fields.next().unwrap();
            assert_eq!(super::city_name(country, id), fields.next());
        }
        assert_eq!(super::city_name("", 0), None);
    }

    #[test]
    fn map_has_distinct_country_shapes_including_small_states() {
        let shapes = super::country_shapes().collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(shapes.len(), super::country_shapes().count());
        assert!(shapes.len() > 220);
        for country in ["US", "PH", "FR", "NO", "SG", "VA", "XK"] {
            assert!(shapes.contains_key(country), "{country}");
        }
        for (code, path) in shapes {
            assert!(code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase()));
            assert!(path.starts_with('M') && path.ends_with('Z'));
        }
    }
}
