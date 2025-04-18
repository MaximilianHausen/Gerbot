#![allow(dead_code)]
pub mod iso_duration {
    use core::fmt;
    use serde::de::Visitor;
    use serde::{de, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(v: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        let mut secs = v.as_secs();
        let hours = secs / 3600;
        secs -= hours * 3600;
        let mins = secs / 60;
        secs -= mins * 60;

        let mut str = "PT".to_owned();
        if hours > 0 {
            str += &format!("H{}", hours);
        }
        if mins > 0 {
            str += &format!("M{}", mins);
        }
        if secs > 0 {
            str += &format!("S{}", secs);
        }

        serializer.serialize_str(&str)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        deserializer.deserialize_str(ISODurationVisitor)
    }

    struct ISODurationVisitor;

    impl Visitor<'_> for ISODurationVisitor {
        type Value = Duration;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            write!(
                formatter,
                "a string containing an iso8601-formatted duration"
            )
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let string = match v.split_once("PT") {
                Some(v) => v.1,
                None => {
                    return Err(de::Error::custom(
                        "no duration specified (does not contain 'PT')",
                    ))
                }
            };
            let mut duration = Duration::default();
            let mut val = 0;

            for c in string.chars() {
                if c.is_ascii_digit() {
                    val = val * 10 + c.to_digit(10).unwrap();
                } else if c == 'H' {
                    duration += Duration::from_secs((3600 * val) as u64);
                    val = 0;
                } else if c == 'M' {
                    duration += Duration::from_secs((60 * val) as u64);
                    val = 0;
                } else if c == 'S' {
                    duration += Duration::from_secs(val as u64);
                    val = 0;
                }
            }

            Ok(duration)
        }
    }
}

pub mod bool_string {
    use core::fmt;
    use serde::de::Visitor;
    use serde::{de, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: bool, serializer: S) -> Result<S::Ok, S::Error> {
        let str = if v { "true" } else { "false" };
        serializer.serialize_str(str)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
        deserializer.deserialize_str(BoolStringVisitor)
    }

    struct BoolStringVisitor;

    impl Visitor<'_> for BoolStringVisitor {
        type Value = bool;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            write!(formatter, "either 'true' or 'false'")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            if v == "true" {
                Ok(true)
            } else if v == "false" {
                Ok(false)
            } else {
                Err(de::Error::custom("not a boolean string"))
            }
        }
    }
}
