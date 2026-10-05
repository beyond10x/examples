//! Zendesk reads through Connectors v0.26.0.
//!
//! Every read goes through the Connectors catalog provider's engine over the Zendesk bundle and
//! selection set that Connectors ships at v0.26.0, vendored under `connectors/` (`PIN.yaml`
//! records the tag, the commit and each file's SHA-256). Two transports serve it:
//!
//! - [`FixtureHttp`]: a fixture Zendesk in this process, serving the synthetic records under
//!   `fixtures/zendesk/` by request path, the way the connectors catalog tests fake the provider's
//!   HTTP port. No account, no network.
//! - [`ConnectorsCli`]: the `connectors` command line against a configured catalog provider and a
//!   connected Zendesk account, as Connectors documents for consumers. The credential stays in
//!   Connectors' custody and never reaches this process.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use connectors_catalog::bundle::Bundle;
use connectors_catalog_provider::{Effect, Engine, Selection};
use connectors_core::ErrorCode;
use connectors_sdk::{AuthenticatedHttp, HttpResponse};
use serde_json::{Value, json};

/// The vendored Zendesk bundle, compiled by Connectors from its pinned Zendesk Support document.
pub const BUNDLE: &str = include_str!("../connectors/zendesk.bundle.json");
/// The vendored selection set: the seven reads Connectors ships for Zendesk.
pub const OPERATIONS: &str = include_str!("../connectors/operations.json");
/// The pin record of the vendored files.
pub const PIN: &str = include_str!("../connectors/PIN.yaml");

/// The fixture records shipped with the crate.
pub fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/zendesk")
}

/// One read's answer: the provider's status and body, and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Read {
    pub status: u16,
    pub body: Value,
    /// The pinned source's SHA-256, from the provider's provenance.
    pub source_revision: String,
    /// The path template the read was bound to.
    pub resource: String,
}

/// Why a read gave no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// Zendesk answered that the record does not exist.
    NotFound,
    /// Anything else, with the reason.
    Failed(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::Failed(reason) => f.write_str(reason),
        }
    }
}

/// Performs one Connectors read operation by its selection id.
pub trait ZendeskReads {
    /// The read `operation` with `input`, one property per path or query parameter.
    ///
    /// # Errors
    /// [`ReadError::NotFound`] for a record Zendesk does not hold, [`ReadError::Failed`] otherwise.
    fn read(&self, operation: &str, input: Value) -> Result<Read, ReadError>;

    /// The read operations this port exposes.
    fn operations(&self) -> Vec<String>;

    /// How the reads travel, for the trail.
    fn transport(&self) -> String;
}

/// The vendored bundle, checked against the digest `PIN.yaml` records for it.
///
/// # Errors
/// When the bundle does not match its pin or does not parse.
pub fn bundle() -> Result<Bundle, String> {
    let pinned = crate::pins::ConnectorsPin::shipped()?;
    let expected = pinned
        .sha256_of("zendesk.bundle.json")
        .ok_or("PIN.yaml names no zendesk.bundle.json")?;
    if crate::json::sha256_hex(BUNDLE.as_bytes()) != expected {
        return Err("the vendored bundle does not match the digest PIN.yaml records".to_owned());
    }
    connectors_core::read_json(BUNDLE.as_bytes())
        .map_err(|error| format!("the vendored bundle does not parse: {error}"))
}

/// The vendored selection set.
///
/// # Errors
/// When it does not parse.
pub fn selections() -> Result<Vec<Selection>, String> {
    let file: Value = serde_json::from_str(OPERATIONS).map_err(|error| error.to_string())?;
    serde_json::from_value(file["operations"].clone()).map_err(|error| error.to_string())
}

/// The catalog provider's engine over the vendored bundle and selection set. Zendesk's paths sit
/// below the authority's root, so the base path is `/`.
///
/// # Errors
/// When the bundle or the selections do not load.
pub fn engine() -> Result<Engine, String> {
    Engine::new(&bundle()?, "/", &selections()?).map_err(|error| error.to_string())
}

/// Reads through the catalog provider's engine, over an HTTP port `H`.
pub struct CatalogReads<H> {
    engine: Engine,
    http: H,
    instance: String,
    runtime: tokio::runtime::Runtime,
}

impl<H: AuthenticatedHttp> CatalogReads<H> {
    /// Reads for the connector instance `instance` over `http`.
    ///
    /// # Errors
    /// When the engine does not load or no runtime can be made.
    pub fn new(http: H, instance: &str) -> Result<Self, String> {
        Ok(Self {
            engine: engine()?,
            http,
            instance: instance.to_owned(),
            runtime: tokio::runtime::Builder::new_current_thread()
                .build()
                .map_err(|error| error.to_string())?,
        })
    }

    /// The HTTP port.
    pub fn http(&self) -> &H {
        &self.http
    }
}

impl<H: AuthenticatedHttp> ZendeskReads for CatalogReads<H> {
    fn read(&self, operation: &str, input: Value) -> Result<Read, ReadError> {
        let answer = self
            .runtime
            .block_on(
                self.engine
                    .read(&self.http, &self.instance, operation, input),
            )
            .map_err(|error| match error.code {
                ErrorCode::NotFound => ReadError::NotFound,
                _ => ReadError::Failed(format!("{operation}: {error}")),
            })?;
        Ok(Read {
            status: answer["status"]
                .as_u64()
                .and_then(|status| u16::try_from(status).ok())
                .unwrap_or_default(),
            source_revision: answer["provenance"]["source_revision"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            resource: answer["provenance"]["resource"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            body: answer["body"].clone(),
        })
    }

    fn operations(&self) -> Vec<String> {
        self.engine
            .declarations(&[Effect::Read])
            .into_iter()
            .map(|operation| operation.id)
            .collect()
    }

    fn transport(&self) -> String {
        "connectors catalog engine".to_owned()
    }
}

/// A fixture Zendesk: answers a GET with the record file at the request path under its root
/// (`api/v2/tickets/1001` reads `api/v2/tickets/1001.json`), or 404. The query is recorded and
/// otherwise ignored: every fixture list fits one page.
pub struct FixtureHttp {
    root: PathBuf,
    requests: Mutex<Vec<String>>,
}

impl FixtureHttp {
    /// A fixture serving the records under `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            requests: Mutex::default(),
        }
    }

    /// Every request served, as `GET <path>?<query>`, in order.
    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl AuthenticatedHttp for FixtureHttp {
    async fn get(
        &self,
        segments: &[&str],
        query: &[(&str, String)],
    ) -> connectors_core::Result<HttpResponse> {
        let path = segments.join("/");
        let query_text: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("GET /{path}?{}", query_text.join("&")));
        let safe = segments
            .iter()
            .all(|segment| !segment.is_empty() && *segment != "." && *segment != "..");
        let file = self.root.join(format!("{path}.json"));
        let (status, body) = match std::fs::read(&file) {
            Ok(body) if safe => (200, body),
            _ => (
                404,
                br#"{"description":"Not found","error":"RecordNotFound"}"#.to_vec(),
            ),
        };
        let mut headers = BTreeMap::new();
        headers.insert("content-type".to_owned(), "application/json".to_owned());
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// Reads through the `connectors` command line: `operations describe` for the operation's schema
/// and revision, then `operations invoke`. A read refused `not_granted`, because the connection's
/// validation evidence (60 seconds by default) lapsed, is retried once after
/// `connections revalidate`. Run against a live account on
/// 2026-10-05 (connectors 0.28.0 with the `zendesk.oauth` client-credentials profile).
pub struct ConnectorsCli {
    program: PathBuf,
    adapter: String,
    connection: String,
    /// `--config` and `--state-dir` for every command, when given.
    globals: Vec<String>,
}

impl ConnectorsCli {
    /// Reads through `program`, the configured catalog adapter `adapter` and the connection
    /// `connection`.
    pub fn new(program: PathBuf, adapter: String, connection: String) -> Self {
        Self {
            program,
            adapter,
            connection,
            globals: Vec::new(),
        }
    }

    /// Selects the `connectors` configuration file and state directory instead of the defaults.
    #[must_use]
    pub fn with_paths(mut self, config: Option<PathBuf>, state_dir: Option<PathBuf>) -> Self {
        for (flag, path) in [("--config", config), ("--state-dir", state_dir)] {
            if let Some(path) = path {
                self.globals.push(flag.to_owned());
                self.globals.push(path.display().to_string());
            }
        }
        self
    }

    /// Renews the connection's validation evidence: a read is refused `not_granted` once it lapsed.
    fn revalidate(&self) -> Result<(), ReadError> {
        let listed = self.run(&["connections", "list", "--adapter", &self.adapter])?;
        let revision = find(&listed, "connections")
            .and_then(Value::as_array)
            .and_then(|connections| {
                connections
                    .iter()
                    .find(|entry| entry["connection"] == self.connection.as_str())
            })
            .and_then(|entry| entry["revision"].as_str())
            .ok_or_else(|| ReadError::Failed(format!("no connection `{}`", self.connection)))?
            .to_owned();
        self.run(&[
            "connections",
            "revalidate",
            "--adapter",
            &self.adapter,
            "--connection",
            &self.connection,
            "--expected-revision",
            &revision,
        ])
        .map(|_| ())
    }

    fn run(&self, arguments: &[&str]) -> Result<Value, ReadError> {
        self.command(arguments).map_err(ReadError::from)
    }

    /// One command; a refusal keeps its failure code, so a caller can act on it.
    fn command(&self, arguments: &[&str]) -> Result<Value, Refusal> {
        let output = Command::new(&self.program)
            .args(["--output", "json"])
            .args(&self.globals)
            .args(arguments)
            .output()
            .map_err(|error| {
                Refusal::Error(ReadError::Failed(format!(
                    "{}: {error}",
                    self.program.display()
                )))
            })?;
        // A refusal is printed on stderr, with stdout empty.
        let printed = if output.stdout.trim_ascii().is_empty() {
            &output.stderr
        } else {
            &output.stdout
        };
        let text = String::from_utf8_lossy(printed);
        let value: Value = serde_json::from_str(text.trim()).map_err(|_| {
            Refusal::Error(ReadError::Failed(format!(
                "connectors answered no JSON: {}",
                text.trim()
            )))
        })?;
        if !output.status.success() {
            return Err(Refusal::Code(
                failure_code(&value).unwrap_or_default().to_owned(),
                value,
            ));
        }
        Ok(value)
    }
}

/// A refused `connectors` command.
enum Refusal {
    /// Refused with this failure code; the whole answer.
    Code(String, Value),
    /// Not run, or answered no JSON.
    Error(ReadError),
}

impl From<Refusal> for ReadError {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::Code(code, _) if code == "not_found" => ReadError::NotFound,
            Refusal::Code(_, value) => ReadError::Failed(format!("connectors refused: {value}")),
            Refusal::Error(error) => error,
        }
    }
}

/// The first member named `name` at the top of `value` or under `data` or `result`.
fn find<'v>(value: &'v Value, name: &str) -> Option<&'v Value> {
    value.get(name).or_else(|| {
        ["data", "result"]
            .iter()
            .find_map(|wrapper| value.get(*wrapper).and_then(|inner| inner.get(name)))
    })
}

/// The failure code of a refused command: `error.data.code` (`{"ok": false, "error":
/// {"code": "failure", "data": {"code": "not_found", …}}}`), else the first `code` found.
fn failure_code(value: &Value) -> Option<&str> {
    value
        .pointer("/error/data/code")
        .or_else(|| find(value, "code"))
        .and_then(Value::as_str)
}

/// The provider answer of `operations invoke`: `{"ok": true, "result": {"adapter", "operation",
/// "revision", "result": "<answer as JSON text>"}}`. A bare object answer is taken as it is.
fn invoked_answer(invoked: &Value) -> Result<Value, ReadError> {
    let payload = invoked
        .pointer("/result/result")
        .or_else(|| find(invoked, "result"))
        .ok_or_else(|| ReadError::Failed("invoke gave no `result`".to_owned()))?;
    match payload {
        Value::String(text) => serde_json::from_str(text)
            .map_err(|_| ReadError::Failed("invoke result is not JSON".to_owned())),
        answer => Ok(answer.clone()),
    }
}

impl ZendeskReads for ConnectorsCli {
    fn read(&self, operation: &str, input: Value) -> Result<Read, ReadError> {
        let described = self.run(&[
            "operations",
            "describe",
            "--adapter",
            &self.adapter,
            "--operation",
            operation,
        ])?;
        let text = |name: &str| {
            find(&described, name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| ReadError::Failed(format!("describe gave no `{name}`")))
        };
        let (schema, revision) = (text("schema")?, text("revision")?);
        let input = input.to_string();
        let invoke = [
            "operations",
            "invoke",
            "--adapter",
            &self.adapter,
            "--connection",
            &self.connection,
            "--operation",
            operation,
            "--schema",
            &schema,
            "--revision",
            &revision,
            "--input-json",
            &input,
        ];
        // Evidence that lapsed is renewed once and the read retried, as Connectors documents
        // for a scheduled caller.
        let invoked = match self.command(&invoke) {
            Err(Refusal::Code(code, _)) if code == "not_granted" => {
                self.revalidate()?;
                self.run(&invoke)?
            }
            answer => answer.map_err(ReadError::from)?,
        };
        let result = invoked_answer(&invoked)?;
        Ok(Read {
            status: result["status"]
                .as_u64()
                .and_then(|status| u16::try_from(status).ok())
                .unwrap_or_default(),
            source_revision: result["provenance"]["source_revision"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            resource: result["provenance"]["resource"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            body: result["body"].clone(),
        })
    }

    fn operations(&self) -> Vec<String> {
        engine()
            .map(|engine| {
                engine
                    .declarations(&[Effect::Read])
                    .into_iter()
                    .map(|operation| operation.id)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn transport(&self) -> String {
        format!("connectors command line, adapter `{}`", self.adapter)
    }
}

/// The fixture transport over the shipped records.
///
/// # Errors
/// When the engine does not load.
pub fn fixture_reads(root: &Path, instance: &str) -> Result<CatalogReads<FixtureHttp>, String> {
    CatalogReads::new(FixtureHttp::new(root), instance)
}

/// A read input of one integer id.
pub fn id_input(name: &str, id: u64) -> Value {
    json!({ name: id })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The shapes `connectors --output json` printed on 2026-10-05 (connectors 0.28.0).

    #[test]
    fn describe_answers_schema_and_revision_under_result() {
        let described = json!({"ok": true, "result": {"adapter": "zendesk",
            "operation": "ticket.show", "revision": "r1", "schema": "s1", "source": "cache",
            "stale": true}});
        assert_eq!(find(&described, "schema"), Some(&json!("s1")));
        assert_eq!(find(&described, "revision"), Some(&json!("r1")));
    }

    #[test]
    fn invoke_answer_is_the_json_text_under_result_result() {
        let answer = json!({"status": 200, "body": {"ticket": {"id": 7}},
            "provenance": {"source_revision": "sha", "resource": "/api/v2/tickets/{ticket_id}"}});
        let invoked = json!({"ok": true, "result": {"adapter": "zendesk",
            "operation": "ticket.show", "revision": "r1", "result": answer.to_string()}});
        assert_eq!(invoked_answer(&invoked), Ok(answer));
    }

    #[test]
    fn a_refusal_code_is_read_from_error_data() {
        let refused = json!({"ok": false, "error": {"code": "failure",
            "data": {"code": "not_found", "kind": "provider"}}});
        assert_eq!(failure_code(&refused), Some("not_found"));
    }
}
