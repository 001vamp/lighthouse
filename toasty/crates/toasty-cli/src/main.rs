//! The `toasty` command: a thin client of toastyd.

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "toasty",
    version,
    about = "Start, follow and answer coding agents"
)]
struct Cli {}

fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    println!(
        "toasty (socket protocol v{})",
        toasty_proto::PROTOCOL_VERSION
    );
    Ok(())
}
