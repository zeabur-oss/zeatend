//! `zeatend`: installs K3s, RKE2 or `KubeSolo` on this Linux host, hardened to the CIS
//! benchmark, from one configuration file.

mod apply;
mod join;
mod kubesolo;
mod load;
mod output;
mod render;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use zeatend_host::HostError;
use zeatend_plan::PlanError;
use zeatend_plan::config::Document;

/// Install K3s, RKE2 or `KubeSolo` on this Linux host, hardened to the CIS benchmark, from one
/// configuration file. A file with a `join` section joins this host to an existing K3s or RKE2
/// cluster instead.
#[derive(Debug, Parser)]
#[command(name = "zeatend", version, about)]
struct Cli {
    /// The configuration file. When it does not exist, the defaults apply: K3s on this host.
    #[arg(long, short, global = true, default_value = load::DEFAULT_CONFIG)]
    config: PathBuf,
    /// The command to run.
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show what `apply` would install and run its host checks, without changing anything.
    Plan {
        /// Print the plan as JSON, without the host checks.
        #[arg(long)]
        json: bool,
    },
    /// Install, or bring an existing install in line with the configuration. Safe to run again.
    Apply,
}

/// Why zeatend stopped.
#[derive(Debug, thiserror::Error)]
enum InstallError {
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error(transparent)]
    Host(#[from] HostError),
    #[error("{0}")]
    Message(String),
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            output::err(&format!("error: {error}"));
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: &Cli) -> Result<(), InstallError> {
    let (document, defaults) = load::document_or_defaults(&cli.config)?;
    if defaults && !matches!(cli.command, Command::Plan { json: true }) {
        output::err(&format!(
            "No configuration at {}; using the defaults: K3s on this host.",
            cli.config.display()
        ));
    }
    match (&cli.command, document) {
        (Command::Plan { json }, Document::Join(config)) => join::show_plan(&config, *json),
        (Command::Apply, Document::Join(config)) => join::apply(&config).await,
        (Command::Plan { json }, Document::Install(config)) => {
            if config.distribution.rancher().is_some() {
                apply::show_plan(&config, *json)
            } else {
                kubesolo::show_plan(&config, *json)
            }
        }
        (Command::Apply, Document::Install(config)) => {
            if config.distribution.rancher().is_some() {
                apply::run(&config).await
            } else {
                kubesolo::apply(&config).await
            }
        }
    }
}

/// Prints `value` as indented JSON.
fn print_json(value: &impl serde::Serialize) -> Result<(), InstallError> {
    let text =
        serde_json::to_string_pretty(value).map_err(|e| InstallError::Message(e.to_string()))?;
    output::line(&text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_and_apply_are_the_commands() {
        let cli = Cli::try_parse_from(["zeatend", "plan", "--json"]).unwrap();
        assert!(matches!(cli.command, Command::Plan { json: true }));
        assert_eq!(cli.config, PathBuf::from(load::DEFAULT_CONFIG));
        let cli = Cli::try_parse_from(["zeatend", "apply", "--config", "/tmp/c.yaml"]).unwrap();
        assert!(matches!(cli.command, Command::Apply));
        assert_eq!(cli.config, PathBuf::from("/tmp/c.yaml"));
        for removed in ["doctor", "node", "uninstall", "gui", "manifests"] {
            assert!(
                Cli::try_parse_from(["zeatend", removed]).is_err(),
                "{removed}"
            );
        }
    }

    #[test]
    fn version_names_the_binary() {
        let err = Cli::try_parse_from(["zeatend", "--version"]).unwrap_err();
        assert_eq!(
            err.to_string().trim_end(),
            format!("zeatend {}", env!("CARGO_PKG_VERSION"))
        );
    }
}
