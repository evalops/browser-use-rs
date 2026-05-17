use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "browser-use-rs")]
#[command(about = "Rust behavioral conformance port of browser-use")]
struct Cli {
    #[arg(long, default_value_t = false)]
    version_target: bool,
}

fn main() {
    let cli = Cli::parse();

    if cli.version_target {
        println!("{}", browser_use_core::INITIAL_UPSTREAM_COMMIT);
    }
}
