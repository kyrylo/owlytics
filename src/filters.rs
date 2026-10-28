//! Dashboard filters. Values are alternatives; different dimensions intersect.
use rusqlite::types::Value;
use serde::{Deserialize, Deserializer, Serialize};

pub const DIMENSIONS: &[(&str, &str, &str)] = &[
    ("page", "path", "Page"),
    ("entry", "entry_page", "Entry page"),
    ("exit", "exit_page", "Exit page"),
    ("source", "source", "Source"),
    ("referrer", "referrer", "Referrer"),
    ("channel", "channel", "Channel"),
    ("utm_medium", "utm_medium", "UTM medium"),
    ("utm_source", "utm_source", "UTM source"),
    ("utm_campaign", "utm_campaign", "UTM campaign"),
    ("utm_content", "utm_content", "UTM content"),
    ("utm_term", "utm_term", "UTM term"),
    ("country", "country", "Country"),
    ("region", "region", "Region"),
    ("city", "city", "City"),
    ("browser", "browser", "Browser"),
    ("browser_version", "browser_version", "Browser version"),
    ("os", "os", "Operating system"),
    ("os_version", "os_version", "OS version"),
    ("device", "device", "Screen size"),
];

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operator {
    #[default]
    Is,
    IsNot,
    Contains,
    ContainsNot,
}
impl Operator {
    pub fn key(self) -> &'static str {
        match self {
            Self::Is => "is",
            Self::IsNot => "is_not",
            Self::Contains => "contains",
            Self::ContainsNot => "contains_not",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Is => "is",
            Self::IsNot => "is not",
            Self::Contains => "contains",
            Self::ContainsNot => "does not contain",
        }
    }
}

pub fn operators(key: &str) -> &'static [Operator] {
    if !matches!(key, "country" | "region" | "city" | "device" | "channel") {
        &[
            Operator::Is,
            Operator::IsNot,
            Operator::Contains,
            Operator::ContainsNot,
        ]
    } else {
        &[Operator::Is, Operator::IsNot]
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub key: String,
    #[serde(default)]
    pub op: Operator,
    pub values: Vec<String>,
}
impl Filter {
    pub fn single(&self) -> Option<&str> {
        (self.op == Operator::Is && self.values.len() == 1).then(|| self.values[0].as_str())
    }
    pub fn matches(&self, value: &str) -> bool {
        let any = self.values.iter().any(|candidate| match self.op {
            Operator::Is | Operator::IsNot => value == candidate,
            Operator::Contains | Operator::ContainsNot => value.contains(candidate),
        });
        if matches!(self.op, Operator::IsNot | Operator::ContainsNot) {
            !any
        } else {
            any
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if !DIMENSIONS.iter().any(|(key, _, _)| *key == self.key) {
            return Err("Choose a valid filter dimension.");
        }
        if !operators(&self.key).contains(&self.op) {
            return Err("Choose a valid operator for this filter.");
        }
        if self.values.is_empty() || self.values.len() > 50 {
            return Err("Choose between one and 50 filter values.");
        }
        if self.values.iter().any(|value| {
            value.len() > 2048
                || value.chars().any(char::is_control)
                || (value.is_empty()
                    && matches!(self.op, Operator::Contains | Operator::ContainsNot))
        }) {
            return Err("Use filter values up to 2,048 bytes without control characters.");
        }
        Ok(())
    }
    pub(crate) fn sql(&self, column: &str, values: &mut Vec<Value>) -> String {
        // Only internal column names enter SQL. Even '%' and '_' are literal
        // substring characters, matching Rust's predicate used for entrances.
        let clauses = self
            .values
            .iter()
            .map(|value| {
                values.push(value.clone().into());
                let parameter = values.len();
                match self.op {
                    Operator::Is | Operator::IsNot => format!("{column}=?{parameter}"),
                    Operator::Contains | Operator::ContainsNot => {
                        format!("instr({column},?{parameter})>0")
                    }
                }
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        format!(
            "{}({clauses})",
            if matches!(self.op, Operator::IsNot | Operator::ContainsNot) {
                "NOT "
            } else {
                ""
            }
        )
    }
}

pub(crate) fn from_query<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Filter>, D::Error> {
    let text = String::deserialize(deserializer)?;
    if text.len() > 8192 {
        return Err(serde::de::Error::custom("Filters are too long."));
    }
    serde_json::from_str(&text).map_err(serde::de::Error::custom)
}
