use std::cmp::Ordering;
use std::fmt::{Debug, Display, Formatter};
use std::hash::{Hash, Hasher};

use semver::Version;
use serde::de::{Error, Visitor};
use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone)]
pub struct LenientVersion(Version, String);

impl LenientVersion {
    pub fn parse(v: &str) -> Result<Self, lenient_semver::parser::Error> {
        lenient_semver::parse(v).map(|parsed| Self(parsed, v.to_string()))
    }
    pub fn inner(&self) -> &Version {
        &self.0
    }

    pub fn original_value(&self) -> &str {
        &self.1
    }

    pub fn clean_original_value(&self) -> &str {
        match self.1.as_bytes()[..2] {
            [b'v', b'0'..=b'9'] => &self.1[1..],
            _ => &self.1,
        }
    }
}

impl Display for LenientVersion {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl PartialEq for LenientVersion {
    fn eq(&self, other: &Self) -> bool {
        PartialEq::eq(&self.0, &other.0)
    }
}

impl Eq for LenientVersion {}

impl PartialOrd for LenientVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        PartialOrd::partial_cmp(&self.0, &other.0)
    }
}

impl Ord for LenientVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        Ord::cmp(&self.0, &other.0)
    }
}

impl Hash for LenientVersion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Hash::hash(&self.0, state);
        Hash::hash(&self.1, state);
    }
}

impl<'de> Deserialize<'de> for LenientVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct VersionDeserializer;
        impl<'de> Visitor<'de> for VersionDeserializer {
            type Value = LenientVersion;

            fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
                formatter.write_str("semver version")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                LenientVersion::parse(v).map_err(Error::custom)
            }
        }
        deserializer.deserialize_str(VersionDeserializer)
    }
}

#[cfg(test)]
mod tests {
    use crate::version::LenientVersion;

    #[test]
    fn test_clean_version() {
        let values = ["v2.0.0", "2.0.0"];
        for value in values {
            assert_eq!(
                "2.0.0",
                LenientVersion::parse(value).unwrap().clean_original_value()
            );
        }
    }
}
