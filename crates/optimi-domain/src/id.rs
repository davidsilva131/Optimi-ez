use std::fmt;

use serde::Serialize;

const MAX_IDENTIFIER_LENGTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdError {
    value: String,
}

impl IdError {
    fn new(value: &str) -> Self {
        Self {
            value: value.to_owned(),
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for IdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid identifier: {}", self.value)
    }
}

impl std::error::Error for IdError {}

fn is_valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_LENGTH
        && value.starts_with(|character: char| character.is_ascii_alphanumeric())
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

macro_rules! identifier_type {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                if is_valid_identifier(&value) {
                    Ok(Self(value))
                } else {
                    Err(IdError::new(&value))
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }
    };
}

identifier_type!(ActionId);
identifier_type!(ProfileId);
identifier_type!(PreviewId);
identifier_type!(TransactionId);
