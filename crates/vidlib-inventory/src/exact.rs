//! Serde adapters for exact decimal JSON quantities. Numeric JSON values, signs,
//! leading zeroes, negative zero and overflow are rejected on import.
use serde::{de::Error, Deserialize, Deserializer};
use std::{fmt::Display, str::FromStr};

fn parse<T: FromStr + Display, E: Error>(value: &str) -> Result<T, E> {
    let parsed: T = value
        .parse()
        .map_err(|_| E::custom("invalid decimal quantity"))?;
    if parsed.to_string() != value {
        return Err(E::custom("decimal quantity must be canonical"));
    }
    Ok(parsed)
}
fn deserialize_number<'de, T: FromStr + Display, D: Deserializer<'de>>(
    d: D,
) -> Result<T, D::Error> {
    parse(&String::deserialize(d)?)
}
fn deserialize_optional<'de, T: FromStr + Display, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<String>::deserialize(d)?
        .map(|s| parse(&s))
        .transpose()
}
macro_rules! adapter {
    ($module:ident, $ty:ty, $optional:ident) => {
        pub mod $module {
            use serde::{Deserializer, Serializer};
            pub fn serialize<S: Serializer>(value: &$ty, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&value.to_string())
            }
            pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<$ty, D::Error> {
                super::deserialize_number(d)
            }
        }
        pub mod $optional {
            use serde::{Deserializer, Serializer};
            pub fn serialize<S: Serializer>(value: &Option<$ty>, s: S) -> Result<S::Ok, S::Error> {
                match value {
                    Some(value) => s.serialize_some(&value.to_string()),
                    None => s.serialize_none(),
                }
            }
            pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<$ty>, D::Error> {
                super::deserialize_optional(d)
            }
        }
    };
}
adapter!(u64_decimal, u64, option_u64_decimal);
adapter!(i128_decimal, i128, option_i128_decimal);
