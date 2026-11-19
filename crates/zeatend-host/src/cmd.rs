//! Running commands.

use std::path::Path;
use std::process::Stdio;

use tokio::io::AsyncWriteExt as _;
use tokio::process::Command;

use crate::HostError;

/// Runs a command, feeding it `stdin`, and returns its standard output.
///
/// # Errors
///
/// When it cannot start or exits with a failure; the error ends with its standard error.
pub async fn run(
    program: impl AsRef<Path>,
    args: &[&str],
    stdin: Option<&[u8]>,
) -> Result<String, HostError> {
    run_with(program, args, &[], stdin).await
}

/// Runs a command with extra environment variables and returns its standard output.
///
/// # Errors
///
/// When it cannot start or exits with a failure; the error ends with its standard error.
pub async fn run_with(
    program: impl AsRef<Path>,
    args: &[&str],
    env: &[(&str, &str)],
    stdin: Option<&[u8]>,
) -> Result<String, HostError> {
    let program = program.as_ref();
    let line = format!("{} {}", program.display(), args.join(" "));
    let fail = |reason: String| HostError::Command {
        command: line.clone(),
        reason,
    };
    let mut child = Command::new(program)
        .args(args)
        .envs(env.iter().copied())
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| fail(e.to_string()))?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(input)
            .await
            .map_err(|e| fail(e.to_string()))?;
        drop(pipe);
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| fail(e.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lines: Vec<&str> = stderr.trim_end().lines().collect();
        let tail = lines
            .get(lines.len().saturating_sub(20)..)
            .unwrap_or_default();
        return Err(fail(format!("{}\n{}", output.status, tail.join("\n"))));
    }
    String::from_utf8(output.stdout).map_err(|e| fail(e.to_string()))
}

/// The standard output and then the standard error of `program --version`, where K3s and RKE2
/// print their release and `KubeSolo` logs it; empty when it cannot run.
pub async fn version_output(program: impl AsRef<Path>) -> String {
    let Ok(output) = Command::new(program.as_ref())
        .arg("--version")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
    else {
        return String::new();
    };
    String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr)
}

/// Whether a command exits successfully; output is discarded.
#[must_use]
pub async fn succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .is_ok_and(|s| s.success())
}
