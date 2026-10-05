#![forbid(unsafe_code)]

//! A stand-in for the ELS registry that Loom's governor reads.
//!
//! Loom pins `b10x-els` at els `ac7dd03`, which predates `support.triage/1`. The workspace
//! `[patch]` replaces that crate with this one, which serves the one protocol this repository
//! needs, vendored byte for byte from ELS commit [`SOURCE_COMMIT`] (`PIN.yaml` records it and its
//! SHA-256; `zendesk-triage pin check` compares the copy with the protocol that ELS commit embeds).
//!
//! The API is the subset of `b10x_els::registry` Loom calls: [`registry::list`],
//! [`registry::get`], [`registry::Builtin`] and [`registry::Error`], with the same shapes.
//!
//! Remove this crate and the `[patch]` once Loom pins an ELS release that carries
//! `support.triage/1`.

/// The ELS commit the vendored protocol was copied from.
pub const SOURCE_COMMIT: &str = "59fd19f639a6427533cd63c5ca53eaaf040d70c7";

/// The vendored protocol document, byte for byte.
pub const SUPPORT_TRIAGE_V1: &str = include_str!("../protocols/support-triage/1.yaml");

/// The pin record, `PIN.yaml`.
pub const PIN: &str = include_str!("../PIN.yaml");

pub mod registry {
    //! The built-in protocols this stand-in serves.

    use std::fmt;

    use b10x_canon::model::{ParseError, Protocol};
    use b10x_canon::validate::Problem;

    /// Every built-in as (name, major, YAML as vendored), sorted by name and major.
    static BUILTINS: &[(&str, u32, &str)] = &[("support-triage", 1, super::SUPPORT_TRIAGE_V1)];

    /// A built-in protocol as vendored, with Canon's validated model of it.
    #[derive(Debug, Clone)]
    pub struct Builtin {
        /// The protocol directory name, `<name>` in `protocols/<name>/<major>.yaml`.
        pub name: &'static str,
        /// The major version.
        pub major: u32,
        /// The document byte for byte as vendored.
        pub yaml: &'static str,
        /// Canon's model of [`Builtin::yaml`], which Canon has validated.
        pub model: Protocol,
    }

    /// Why [`get`] refused.
    #[derive(Debug)]
    pub enum Error {
        /// No built-in has this name.
        UnknownName { name: String, major: u32 },
        /// A built-in has this name, but not at this major.
        UnknownMajor { name: String, major: u32 },
        /// Canon cannot parse the embedded document.
        Parse {
            name: &'static str,
            major: u32,
            error: ParseError,
        },
        /// Canon parses the embedded document and finds it invalid.
        Invalid {
            name: &'static str,
            major: u32,
            problems: Vec<Problem>,
        },
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Error::UnknownName { name, major } => {
                    write!(f, "{name}@{major}: no built-in protocol is named `{name}`")
                }
                Error::UnknownMajor { name, major } => {
                    write!(
                        f,
                        "{name}@{major}: built-in `{name}` has no major version {major}"
                    )
                }
                Error::Parse { name, major, error } => {
                    write!(f, "{name}@{major}: Canon cannot parse it: {error}")
                }
                Error::Invalid {
                    name,
                    major,
                    problems,
                } => {
                    write!(f, "{name}@{major}: Canon finds it invalid:")?;
                    for problem in problems {
                        write!(f, "\n  {problem}")?;
                    }
                    Ok(())
                }
            }
        }
    }

    impl std::error::Error for Error {}

    /// Every built-in, as (name, major), sorted by name and major.
    pub fn list() -> Vec<(&'static str, u32)> {
        BUILTINS
            .iter()
            .map(|(name, major, _)| (*name, *major))
            .collect()
    }

    /// The built-in `name@major`, validated by Canon.
    pub fn get(name: &str, major: u32) -> Result<Builtin, Error> {
        let Some((name, major, yaml)) = BUILTINS
            .iter()
            .find(|(known, version, _)| *known == name && *version == major)
            .copied()
        else {
            let name = name.to_owned();
            return Err(if BUILTINS.iter().any(|(known, _, _)| *known == name) {
                Error::UnknownMajor { name, major }
            } else {
                Error::UnknownName { name, major }
            });
        };
        let model =
            b10x_canon::model::parse(yaml).map_err(|error| Error::Parse { name, major, error })?;
        b10x_canon::validate::validate(&model).map_err(|problems| Error::Invalid {
            name,
            major,
            problems,
        })?;
        Ok(Builtin {
            name,
            major,
            yaml,
            model,
        })
    }
}
