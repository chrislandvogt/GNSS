use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ModelError;

/// The longest id accepted. RINEX's marker name field is 60 characters.
const MAX_LEN: usize = 60;

/// Ids name files and appear in reports and other software's formats, so
/// they're kept to characters that are safe everywhere: letters, digits and
/// `-`, `_` and `.`, not starting with `.`.
fn check(kind: &str, text: &str) -> Result<(), ModelError> {
    if text.is_empty() {
        return Err(ModelError::Invalid(format!("{kind} id is empty")));
    }
    if text.len() > MAX_LEN {
        return Err(ModelError::Invalid(format!(
            "{kind} id {text:?} is longer than {MAX_LEN} characters"
        )));
    }
    if text.starts_with('.') {
        return Err(ModelError::Invalid(format!(
            "{kind} id {text:?} can't start with '.'"
        )));
    }
    if let Some(c) = text
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        return Err(ModelError::Invalid(format!(
            "{kind} id {text:?} has {c:?}; use letters, digits, '-', '_' and '.'"
        )));
    }
    Ok(())
}

macro_rules! id {
    ($name:ident, $kind:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// An id from its text, which must be 1 to 60 letters, digits,
            /// `-`, `_` or `.`, not starting with `.`.
            pub fn new(text: impl Into<String>) -> Result<Self, ModelError> {
                let text = text.into();
                check($kind, &text)?;
                Ok(Self(text))
            }

            /// The id's text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({
                    "type": "string",
                    "description": $doc,
                    "pattern": "^[A-Za-z0-9_-][A-Za-z0-9._-]{0,59}$"
                })
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                $name::new(String::deserialize(d)?).map_err(serde::de::Error::custom)
            }
        }
    };
}

id!(MarkId, "mark", "A survey mark's id, such as `PM12345`.");
id!(
    OverlayId,
    "overlay",
    "An overlay's id, such as `2026-10-15-PM12345A-antenna`. It's also its file name in `overlays/`."
);
id!(
    OccupationId,
    "occupation",
    "An occupation's id, such as `2026-10-14-PM12345-A`. It's also its file name in `occupations/`."
);

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{MarkId, OccupationId};

    #[test]
    fn ids_are_safe_in_file_names_and_other_formats() {
        assert_eq!(MarkId::new("PM12345").unwrap().as_str(), "PM12345");
        assert!(OccupationId::new("2026-10-14-PM12345-A").is_ok());
        assert!(MarkId::new("SSM_102.3").is_ok());
        for bad in [
            "",
            ".hidden",
            "PM 1",
            "a/b",
            "a\\b",
            "PM1:2",
            &"X".repeat(61),
        ] {
            let err = MarkId::new(bad).unwrap_err().to_string();
            assert!(err.starts_with("mark id"), "{bad:?}: {err}");
        }
    }
}
