//! `zeatend`: installs K3s, RKE2 or `KubeSolo` on this Linux host, hardened to the CIS
//! benchmark, from one configuration file.

mod apply;
mod doctor;
mod i18n;
mod join;
mod kubesolo;
mod load;
mod output;
mod render;
mod uninstall;
mod unit;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use zeatend_host::HostError;
use zeatend_host::progress::Event;
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
    ///
    /// On a host with systemd the install runs in the unit `zeatend-install-<id>.service` and
    /// goes on when this session ends. Running `apply` again while it runs shows its progress
    /// instead of starting another install.
    Apply {
        /// Run the install in this process, not in a systemd unit. It stops when the session ends.
        #[arg(long)]
        foreground: bool,
        /// Print the progress as JSON lines on stdout.
        #[arg(long)]
        json: bool,
    },
    /// Remove K3s, RKE2 or KubeSolo and zeatend's files from this host. Volumes stay. Safe to run
    /// again.
    Uninstall {
        /// Remove without asking first.
        #[arg(long, short)]
        yes: bool,
        /// Also remove the volumes: the local-path storage of K3s and KubeSolo,
        /// /var/lib/zeabur/storage and /var/lib/longhorn.
        #[arg(long)]
        delete_volumes: bool,
    },
    /// Check this host and the cluster on it. Exits 0 when all is well, 1 for a warning, 2 for
    /// something critical and 3 when the checks cannot run, as monitoring plugins do.
    Doctor,
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
    /// The person stopped following the install unit, which goes on.
    #[error(
        "stopped following {0}; the install goes on. Run `sudo zeatend apply` to follow it again."
    )]
    Detached(String),
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    if matches!(cli.command, Command::Doctor) {
        return doctor::run(&cli.config).await;
    }
    if let Command::Apply { json: true, .. } = cli.command {
        output::print_json_events();
    }
    match run(&cli).await {
        Ok(()) => {
            output::emit(&Event::Done);
            ExitCode::SUCCESS
        }
        Err(error @ InstallError::Detached(_)) => {
            output::err(&error.to_string());
            // 128 + SIGINT, as a shell reports Ctrl-C.
            ExitCode::from(130)
        }
        Err(error) => {
            output::emit(&Event::Failed {
                error: error.to_string(),
            });
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: &Cli) -> Result<(), InstallError> {
    if let Command::Uninstall {
        yes,
        delete_volumes,
    } = cli.command
    {
        return uninstall::run(yes, delete_volumes).await;
    }
    if let Command::Apply {
        foreground: false, ..
    } = cli.command
        && zeatend_host::systemd::booted()
    {
        return unit::apply(&cli.config).await;
    }
    let (document, defaults) = load::document_or_defaults(&cli.config)?;
    if defaults && !matches!(cli.command, Command::Plan { json: true }) {
        output::note(&format!(
            "No configuration at {}; using the defaults: K3s on this host.",
            cli.config.display()
        ));
    }
    match (&cli.command, document) {
        (Command::Plan { json }, Document::Join(config)) => join::show_plan(&config, *json).await,
        (Command::Apply { .. }, Document::Join(config)) => join::apply(&config).await,
        (Command::Plan { json }, Document::Install(config)) => {
            if config.distribution.rancher().is_some() {
                apply::show_plan(&config, *json).await
            } else {
                kubesolo::show_plan(&config, *json).await
            }
        }
        (Command::Apply { .. }, Document::Install(config)) => {
            if config.distribution.rancher().is_some() {
                apply::run(&config).await
            } else {
                kubesolo::apply(&config).await
            }
        }
        (Command::Uninstall { .. } | Command::Doctor, _) => Ok(()),
    }
}

/// Prints `value` as indented JSON.
fn print_json(value: &impl serde::Serialize) -> Result<(), InstallError> {
    let text =
        serde_json::to_string_pretty(value).map_err(|e| InstallError::Message(e.to_string()))?;
    output::stdout(&text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_apply_uninstall_and_doctor_are_the_commands() {
        let cli = Cli::try_parse_from(["zeatend", "plan", "--json"]).unwrap();
        assert!(matches!(cli.command, Command::Plan { json: true }));
        assert_eq!(cli.config, PathBuf::from(load::DEFAULT_CONFIG));
        let cli = Cli::try_parse_from(["zeatend", "apply", "--config", "/tmp/c.yaml"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Apply {
                foreground: false,
                json: false
            }
        ));
        assert_eq!(cli.config, PathBuf::from("/tmp/c.yaml"));
        let cli =
            Cli::try_parse_from(unit::command("/usr/local/bin/zeatend", "/etc/z.yaml")).unwrap();
        assert!(matches!(
            cli.command,
            Command::Apply {
                foreground: true,
                json: true
            }
        ));
        assert_eq!(cli.config, PathBuf::from("/etc/z.yaml"));

        let cli =
            Cli::try_parse_from(["zeatend", "uninstall", "--yes", "--delete-volumes"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Uninstall {
                yes: true,
                delete_volumes: true
            }
        ));
        let cli = Cli::try_parse_from(["zeatend", "uninstall"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Uninstall {
                yes: false,
                delete_volumes: false
            }
        ));
        let cli = Cli::try_parse_from(["zeatend", "doctor"]).unwrap();
        assert!(matches!(cli.command, Command::Doctor));
        for removed in ["node", "gui", "manifests"] {
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
