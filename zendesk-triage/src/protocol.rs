//! The protocol the triage runs under, `support.triage/1`.
//!
//! The governor reads protocols from the ELS registry by `<name>@<major>`. Loom pins an ELS that
//! predates this protocol, so the workspace patches that registry with `vendor/els`, which serves
//! the copy vendored from ELS commit [`b10x_els::SOURCE_COMMIT`]. This module reads it through the
//! same registry the governor uses, so what the demo prints is what the governor evaluated.

use b10x_canon::ir::{Ir, compile};

/// The registry name the case is opened on.
pub const PROTOCOL: &str = "support-triage@1";

/// The protocol's document, byte for byte as the governor reads it.
pub fn document() -> &'static str {
    b10x_els::SUPPORT_TRIAGE_V1
}

/// The protocol compiled by Canon, read through the registry the governor reads.
///
/// # Errors
/// When the registry refuses the protocol or Canon does not compile it.
pub fn compiled() -> Result<Ir, String> {
    let builtin =
        b10x_els::registry::get("support-triage", 1).map_err(|error| error.to_string())?;
    compile(&builtin.model).map_err(|problems| {
        problems
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    })
}

/// The first twelve hex digits of the SHA-256 of the document, for the trail.
pub fn short_digest() -> String {
    crate::json::sha256_hex(document().as_bytes())[..12].to_owned()
}
