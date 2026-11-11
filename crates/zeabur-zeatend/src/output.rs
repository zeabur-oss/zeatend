//! The only place that writes to stdout.

#[expect(
    clippy::print_stdout,
    reason = "the output module is where the binary prints"
)]
pub(crate) fn line(text: &str) {
    println!("{text}");
}
