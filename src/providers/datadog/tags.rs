use serde::Serialize;
use std::fmt;

/// A comma-separated collection of Datadog tags.
///
/// Datadog recommends `key:value` tags. Values are kept verbatim after their
/// syntax and length have been validated.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Tags(String);

impl Tags {
    pub const fn new() -> Self {
        Self(String::new())
    }

    /// Appends a `key:value` tag.
    pub fn push(&mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> Result<(), TagError> {
        let key = key.as_ref();
        let value = value.as_ref();
        validate_tag(key, value)?;
        self.push_separator();
        self.0.push_str(key);
        self.0.push(':');
        self.0.push_str(value);
        Ok(())
    }

    /// Appends a tag without a value.
    pub fn push_bare(&mut self, tag: impl AsRef<str>) -> Result<(), TagError> {
        let tag = tag.as_ref();
        validate_bare_tag(tag)?;
        self.push_separator();
        self.0.push_str(tag);
        Ok(())
    }

    pub fn with(mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> Result<Self, TagError> {
        self.push(key, value)?;
        Ok(self)
    }

    pub fn with_bare(mut self, tag: impl AsRef<str>) -> Result<Self, TagError> {
        self.push_bare(tag)?;
        Ok(self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn push_separator(&mut self) {
        if !self.0.is_empty() {
            self.0.push(',');
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TagError {
    Empty,
    MustStartWithLetter,
    InvalidCharacter { part: &'static str, character: char },
    TooLong { characters: usize },
}

impl fmt::Display for TagError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("tag must not be empty"),
            Self::MustStartWithLetter => formatter.write_str("tag must start with a letter"),
            Self::InvalidCharacter { part, character } => {
                write!(
                    formatter,
                    "tag {part} contains invalid character {character:?}"
                )
            }
            Self::TooLong { characters } => {
                write!(formatter, "tag has {characters} characters; maximum is 200")
            }
        }
    }
}

impl std::error::Error for TagError {}

fn validate_tag(key: &str, value: &str) -> Result<(), TagError> {
    validate_start(key)?;
    validate_characters("key", key, false)?;
    validate_characters("value", value, true)?;
    validate_length(key.chars().count() + 1 + value.chars().count())
}

fn validate_bare_tag(tag: &str) -> Result<(), TagError> {
    validate_start(tag)?;
    validate_characters("value", tag, true)?;
    validate_length(tag.chars().count())
}

fn validate_start(tag: &str) -> Result<(), TagError> {
    match tag.chars().next() {
        None => Err(TagError::Empty),
        Some(character) if character.is_alphabetic() => Ok(()),
        Some(_) => Err(TagError::MustStartWithLetter),
    }
}

fn validate_characters(part: &'static str, input: &str, allow_colon: bool) -> Result<(), TagError> {
    if let Some(character) = input.chars().find(|character| {
        !(character.is_alphanumeric()
            || matches!(character, '_' | '-' | '.' | '/' | '@')
            || (allow_colon && *character == ':'))
    }) {
        return Err(TagError::InvalidCharacter { part, character });
    }
    Ok(())
}

fn validate_length(characters: usize) -> Result<(), TagError> {
    if characters > 200 {
        Err(TagError::TooLong { characters })
    } else {
        Ok(())
    }
}

impl fmt::Display for Tags {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tags_are_encoded_as_a_comma_separated_string() {
        let mut tags = Tags::new().with("env", "production").unwrap();
        tags.push("version", "1.2.3").unwrap();
        tags.push_bare("canary").unwrap();

        assert_eq!(tags.as_str(), "env:production,version:1.2.3,canary");
        assert_eq!(
            serde_json::to_value(tags).unwrap(),
            json!("env:production,version:1.2.3,canary")
        );
    }

    #[test]
    fn rejects_invalid_tags_without_mutating_the_collection() {
        let mut tags = Tags::new().with("env", "production").unwrap();

        assert_eq!(
            tags.push("release", "stable,admin:true"),
            Err(TagError::InvalidCharacter {
                part: "value",
                character: ',',
            })
        );
        assert_eq!(
            tags.push("invalid:key", "value"),
            Err(TagError::InvalidCharacter {
                part: "key",
                character: ':',
            })
        );
        assert_eq!(
            tags.push("2invalid", "value"),
            Err(TagError::MustStartWithLetter)
        );
        assert_eq!(tags.as_str(), "env:production");
    }

    #[test]
    fn rejects_tags_longer_than_two_hundred_characters() {
        let value = "x".repeat(197);
        assert_eq!(
            Tags::new().with("env", value),
            Err(TagError::TooLong { characters: 201 })
        );
    }
}
