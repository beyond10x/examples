#![forbid(unsafe_code)]

//! `zendesk-triage`: triage support tickets locally and print the decision trail.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use zendesk_triage::composition::Composition;
use zendesk_triage::zendesk::{ConnectorsCli, ZendeskReads, fixture_reads, fixture_root};
use zendesk_triage::{conform, pins, protocol, time, trail, triage};

#[derive(Parser)]
#[command(name = "zendesk-triage", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Triage tickets and print each decision trail.
    Triage(TriageArgs),
    /// Run the ESS suite against the fixture Zendesk and write `ess-conformance-results/1`.
    Conform {
        /// The suite `ess verify conform author` wrote.
        #[arg(long)]
        suite: PathBuf,
        /// Where to write the results.
        #[arg(long)]
        results: PathBuf,
    },
    /// Check the vendored protocol and Connectors files against their pins.
    Pins {
        /// The workspace manifest `cargo metadata` reads to locate the fetched Connectors checkout.
        #[arg(long)]
        manifest_path: Option<PathBuf>,
    },
    /// Print the binding of each protocol action and check it against the protocol and Connectors.
    Bindings,
}

#[derive(Clone, Copy, ValueEnum)]
enum Source {
    /// The fixture Zendesk shipped under `fixtures/zendesk`, read through the Connectors engine.
    Fixture,
    /// A connected Zendesk account through the `connectors` command line.
    Connectors,
}

#[derive(clap::Args)]
struct TriageArgs {
    /// Ticket ids. With the fixture and none given, every fixture ticket.
    tickets: Vec<String>,
    /// Approve a capability the policy holds for a person, for this run (repeatable).
    #[arg(long = "approve", value_name = "CAPABILITY")]
    approvals: Vec<String>,
    /// Where tickets are read from.
    #[arg(long, value_enum, default_value_t = Source::Fixture)]
    zendesk: Source,
    /// The fixture records' directory.
    #[arg(long, value_name = "DIR")]
    fixtures: Option<PathBuf>,
    /// The `connectors` executable.
    #[arg(long, value_name = "PATH", default_value = "connectors")]
    connectors_bin: PathBuf,
    /// The `connectors` configuration file, when not its default.
    #[arg(long, value_name = "PATH")]
    connectors_config: Option<PathBuf>,
    /// The `connectors` state directory, when not its default.
    #[arg(long, value_name = "DIR")]
    connectors_state_dir: Option<PathBuf>,
    /// The configured catalog adapter alias for Zendesk.
    #[arg(long, value_name = "ALIAS", default_value = "zendesk")]
    adapter: String,
    /// The connection reference `connectors connections connect` printed.
    #[arg(long, value_name = "REF")]
    connection: Option<String>,
    /// The instant the run reads as now (RFC 3339, UTC). Fixture default 2026-10-05T10:00:00Z;
    /// otherwise the system clock.
    #[arg(long, value_name = "INSTANT")]
    now: Option<String>,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("zendesk-triage: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, String> {
    let composition = Composition::shipped()?;
    match cli.command {
        Command::Triage(arguments) => triage_tickets(&composition, arguments),
        Command::Conform { suite, results } => {
            let suite = conform::read_suite(&suite)?;
            let reads = fixture_reads(&fixture_root(), &composition.connector.instance)?;
            let verdicts = conform::run(&suite, &reads, &composition)?;
            let mut failed = 0;
            for verdict in &verdicts {
                println!(
                    "{:<11} {}{}",
                    verdict.status,
                    verdict.scenario,
                    verdict
                        .message
                        .as_ref()
                        .map_or(String::new(), |message| format!(": {message}"))
                );
                if verdict.status != "passed" {
                    failed += 1;
                }
            }
            let completed_at = u64::try_from(time::now()).unwrap_or_default() * 1_000;
            let document = conform::results(&verdicts, completed_at);
            std::fs::write(
                &results,
                serde_json::to_string_pretty(&document).map_err(|error| error.to_string())? + "\n",
            )
            .map_err(|error| format!("{}: {error}", results.display()))?;
            println!("{} scenario(s), {failed} not passed", verdicts.len());
            Ok(if failed == 0 && !verdicts.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Pins { manifest_path } => {
            for line in pins::check_protocol()? {
                println!("ok  {line}");
            }
            for line in pins::check_connectors(manifest_path.as_deref())? {
                println!("ok  {line}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Bindings => {
            let ir = protocol::compiled()?;
            let reads: Vec<String> = zendesk_triage::zendesk::engine()?
                .declarations(&[connectors_catalog_provider::Effect::Read])
                .into_iter()
                .map(|operation| operation.id)
                .collect();
            for (action, declared) in &ir.actions {
                let effect = declared
                    .effect
                    .as_ref()
                    .map_or("unspecified", |effect| effect.as_str());
                let capability = declared
                    .requires
                    .first()
                    .map_or(String::from("-"), |c| c.as_str().to_owned());
                let policy = composition
                    .policy_for(&capability)
                    .map_or(String::from("-"), |p| format!("{p:?}"));
                println!(
                    "{:<22} effect {:<6} capability {:<22} policy {:<16} {:?}",
                    action.as_str(),
                    effect,
                    capability,
                    policy,
                    composition.bindings.get(action.as_str())
                );
            }
            let problems = composition.problems(&ir, &reads);
            for problem in &problems {
                println!("problem: {problem}");
            }
            Ok(if problems.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
    }
}

fn triage_tickets(composition: &Composition, arguments: TriageArgs) -> Result<ExitCode, String> {
    let approvals: BTreeSet<String> = arguments.approvals.into_iter().collect();
    let (reads, now, mut tickets): (Box<dyn ZendeskReads>, i64, Vec<String>) = match arguments
        .zendesk
    {
        Source::Fixture => {
            let root = arguments.fixtures.unwrap_or_else(fixture_root);
            let tickets = fixture_tickets(&root)?;
            (
                Box::new(fixture_reads(&root, &composition.connector.instance)?),
                triage::FIXTURE_NOW,
                tickets,
            )
        }
        Source::Connectors => {
            let connection = arguments
                .connection
                .ok_or("--zendesk connectors needs --connection")?;
            (
                Box::new(
                    ConnectorsCli::new(arguments.connectors_bin, arguments.adapter, connection)
                        .with_paths(arguments.connectors_config, arguments.connectors_state_dir),
                ),
                time::now(),
                Vec::new(),
            )
        }
    };
    if !arguments.tickets.is_empty() {
        tickets = arguments.tickets;
    }
    if tickets.is_empty() {
        return Err("name at least one ticket id".to_owned());
    }
    let now = match arguments.now {
        Some(text) => {
            time::parse(&text).ok_or_else(|| format!("`{text}` is not an RFC 3339 UTC instant"))?
        }
        None => now,
    };
    println!(
        "reads: {}; now {}; approvals: {}\n",
        reads.transport(),
        time::format(now),
        if approvals.is_empty() {
            "none".to_owned()
        } else {
            approvals.iter().cloned().collect::<Vec<_>>().join(", ")
        }
    );
    let options = triage::Options { approvals, now };
    let mut done = Vec::new();
    for ticket in &tickets {
        let triage = triage::run(reads.as_ref(), composition, ticket, &options)?;
        println!("{}", trail::render(&triage));
        done.push(triage);
    }
    print!("{}", trail::summary(&done));
    Ok(ExitCode::SUCCESS)
}

/// The ids of the fixture tickets under `root`, in numeric order.
fn fixture_tickets(root: &std::path::Path) -> Result<Vec<String>, String> {
    let directory = root.join("api/v2/tickets");
    let mut ids: Vec<u64> = std::fs::read_dir(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".json"))
                .and_then(|id| id.parse().ok())
        })
        .collect();
    ids.sort_unstable();
    Ok(ids.into_iter().map(|id| id.to_string()).collect())
}
