//! Zeabur Zeatend, a Kubernetes cluster manager for K3s, RKE2 and KubeSolo over SSH.

mod output;

fn main() {
    output::line(&version_line());
}

/// The line `--version` style output prints.
fn version_line() -> String {
    format!("zeabur-zeatend {}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::version_line;

    #[test]
    fn version_line_names_the_binary_and_its_version() {
        assert_eq!(
            version_line(),
            format!("zeabur-zeatend {}", env!("CARGO_PKG_VERSION"))
        );
    }
}
