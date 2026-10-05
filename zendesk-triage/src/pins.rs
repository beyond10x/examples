//! The pin checks: each vendored file is the file of the commit its `PIN.yaml` names.
//!
//! - `vendor/els/protocols/support-triage/1.yaml` against the protocol ELS embeds at the pinned
//!   commit, read through `b10x-canon-engineering` at that commit (a Cargo dependency of this
//!   crate), and against the SHA-256 `vendor/els/PIN.yaml` records.
//! - `connectors/*.json` against the SHA-256 `connectors/PIN.yaml` records, and, when
//!   `cargo metadata` can locate the Connectors checkout Cargo fetched for the pinned tag, against
//!   the files of that checkout byte for byte.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use serde_json::Value;

use crate::json::sha256_hex;

/// `vendor/els/PIN.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolPin {
    pub format: String,
    pub protocol: String,
    pub file: String,
    pub source: ProtocolSource,
    pub sha256: String,
}

/// Where the vendored protocol was copied from.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolSource {
    pub repository: String,
    pub commit: String,
    pub path: String,
}

impl ProtocolPin {
    /// The shipped pin.
    ///
    /// # Errors
    /// When it does not parse.
    pub fn shipped() -> Result<Self, String> {
        serde_yaml_ng::from_str(b10x_els::PIN)
            .map_err(|error| format!("vendor/els/PIN.yaml: {error}"))
    }
}

/// `connectors/PIN.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorsPin {
    pub format: String,
    pub provider: String,
    pub source: ConnectorsSource,
    pub files: Vec<PinnedFile>,
}

/// The Connectors release the files were copied from.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorsSource {
    pub repository: String,
    pub tag: String,
    pub commit: String,
}

/// One vendored file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedFile {
    /// Its name under `connectors/`.
    pub file: String,
    /// Its path in the Connectors repository.
    pub path: String,
    pub sha256: String,
}

impl ConnectorsPin {
    /// The shipped pin.
    ///
    /// # Errors
    /// When it does not parse.
    pub fn shipped() -> Result<Self, String> {
        serde_yaml_ng::from_str(crate::zendesk::PIN)
            .map_err(|error| format!("connectors/PIN.yaml: {error}"))
    }

    /// The recorded SHA-256 of the vendored file `file`.
    pub fn sha256_of(&self, file: &str) -> Option<&str> {
        self.files
            .iter()
            .find(|pinned| pinned.file == file)
            .map(|pinned| pinned.sha256.as_str())
    }
}

/// The vendored bytes of a file under `connectors/`.
fn connectors_bytes(file: &str) -> Option<&'static str> {
    match file {
        "zendesk.bundle.json" => Some(crate::zendesk::BUNDLE),
        "operations.json" => Some(crate::zendesk::OPERATIONS),
        _ => None,
    }
}

/// The manifest of this crate, to read the revision it names for ELS.
const MANIFEST: &str = include_str!("../Cargo.toml");

/// Checks the vendored protocol. Returns one line per check that held.
///
/// # Errors
/// The first check that failed.
pub fn check_protocol() -> Result<Vec<String>, String> {
    let pin = ProtocolPin::shipped()?;
    let mut held = Vec::new();
    if pin.format != "examples-vendored-protocol/1" || pin.protocol != crate::protocol::PROTOCOL {
        return Err(format!(
            "vendor/els/PIN.yaml pins `{}` in `{}`",
            pin.protocol, pin.format
        ));
    }
    let vendored = crate::protocol::document();
    let digest = sha256_hex(vendored.as_bytes());
    if digest != pin.sha256 {
        return Err(format!(
            "vendor/els/{} has SHA-256 {digest}, PIN.yaml records {}",
            pin.file, pin.sha256
        ));
    }
    held.push(format!("vendor/els/{} sha256 {digest}", pin.file));
    if pin.source.commit != b10x_els::SOURCE_COMMIT {
        return Err(format!(
            "PIN.yaml names ELS {}, the stand-in registry says {}",
            pin.source.commit,
            b10x_els::SOURCE_COMMIT
        ));
    }
    let dependency = MANIFEST
        .lines()
        .find(|line| line.starts_with("b10x-canon-engineering "))
        .ok_or("Cargo.toml has no b10x-canon-engineering dependency")?;
    if !dependency.contains(&format!("rev = \"{}\"", pin.source.commit)) {
        return Err(format!(
            "Cargo.toml pins b10x-canon-engineering other than ELS {}: {dependency}",
            pin.source.commit
        ));
    }
    let released = canon_engineering::registry::get("support-triage", 1)
        .map_err(|error| format!("ELS {} has no support-triage@1: {error}", pin.source.commit))?;
    if released.yaml != vendored {
        return Err(format!(
            "vendor/els/{} differs from {} at ELS {}",
            pin.file, pin.source.path, pin.source.commit
        ));
    }
    held.push(format!(
        "vendor/els/{} is {} at {} {}",
        pin.file, pin.source.path, pin.source.repository, pin.source.commit
    ));
    let ir = crate::protocol::compiled()?;
    let report = b10x_canon::check::check(&ir, None).map_err(|refusal| refusal.to_string())?;
    if !report.is_clean() {
        return Err(format!("canon check is not clean:\n{report}"));
    }
    held.push(format!("canon check clean: {} states", report.states));
    Ok(held)
}

/// Checks the vendored Connectors files. Returns one line per check that held.
///
/// # Errors
/// The first check that failed.
pub fn check_connectors(manifest: Option<&Path>) -> Result<Vec<String>, String> {
    let pin = ConnectorsPin::shipped()?;
    let mut held = Vec::new();
    if pin.format != "examples-vendored-connectors/1" || pin.provider != "zendesk" {
        return Err("connectors/PIN.yaml is not a zendesk pin".to_owned());
    }
    for pinned in &pin.files {
        let bytes = connectors_bytes(&pinned.file)
            .ok_or_else(|| format!("connectors/{} is not vendored", pinned.file))?;
        let digest = sha256_hex(bytes.as_bytes());
        if digest != pinned.sha256 {
            return Err(format!(
                "connectors/{} has SHA-256 {digest}, PIN.yaml records {}",
                pinned.file, pinned.sha256
            ));
        }
        held.push(format!("connectors/{} sha256 {digest}", pinned.file));
    }
    let reads = crate::zendesk::engine()?
        .declarations(&[connectors_catalog_provider::Effect::Read])
        .len();
    held.push(format!("connectors engine loads the bundle: {reads} reads"));
    match fetched_checkout(manifest, &pin)? {
        Some(root) => {
            for pinned in &pin.files {
                let upstream = std::fs::read(root.join(&pinned.path))
                    .map_err(|error| format!("{}: {error}", root.join(&pinned.path).display()))?;
                let vendored = connectors_bytes(&pinned.file).unwrap_or_default();
                if upstream != vendored.as_bytes() {
                    return Err(format!(
                        "connectors/{} differs from {} at {} {}",
                        pinned.file, pinned.path, pin.source.tag, pin.source.commit
                    ));
                }
                held.push(format!(
                    "connectors/{} is {} at {} {}",
                    pinned.file, pinned.path, pin.source.tag, pin.source.commit
                ));
            }
        }
        None => held.push(
            "the fetched Connectors checkout was not located; compared digests only".to_owned(),
        ),
    }
    Ok(held)
}

/// The root of the Connectors checkout Cargo fetched for this workspace, located through
/// `cargo metadata`; refused when Cargo resolved Connectors to another tag or commit than the pin.
fn fetched_checkout(
    manifest: Option<&Path>,
    pin: &ConnectorsPin,
) -> Result<Option<PathBuf>, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command.args(["metadata", "--format-version", "1", "--locked"]);
    if let Some(manifest) = manifest {
        command.arg("--manifest-path").arg(manifest);
    }
    let Ok(output) = command.output() else {
        return Ok(None);
    };
    if !output.status.success() {
        return Ok(None);
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("cargo metadata: {error}"))?;
    let Some(package) = metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|package| package["name"] == "connectors-catalog-provider")
    else {
        return Ok(None);
    };
    let source = package["source"].as_str().unwrap_or_default();
    let expected = format!(
        "git+{}?tag={}#{}",
        pin.source.repository, pin.source.tag, pin.source.commit
    );
    if source != expected {
        return Err(format!(
            "Cargo resolves connectors-catalog-provider from `{source}`, the pin is `{expected}`"
        ));
    }
    let manifest_path = PathBuf::from(package["manifest_path"].as_str().unwrap_or_default());
    // The provider's manifest is `adapters/catalog/Cargo.toml` in the Connectors repository.
    Ok(manifest_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf))
}
