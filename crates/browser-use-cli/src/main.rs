use clap::Parser;
use schemars::schema_for;

#[derive(Debug, Parser)]
#[command(name = "browser-use-rs")]
#[command(about = "Rust behavioral conformance port of browser-use")]
struct Cli {
    #[arg(long, default_value_t = false)]
    version_target: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Print the frozen upstream browser-use commit target.
    VersionTarget,
    /// Print a JSON Schema for a compatibility contract.
    Schema { contract: SchemaContract },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum SchemaContract {
    Action,
    AgentOutput,
    BrowserState,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::VersionTarget) => {
            println!("{}", browser_use_core::INITIAL_UPSTREAM_COMMIT);
        }
        Some(Command::Schema { contract }) => {
            let schema = match contract {
                SchemaContract::Action => schema_for!(browser_use_tools::BrowserAction),
                SchemaContract::AgentOutput => schema_for!(browser_use_core::AgentOutput),
                SchemaContract::BrowserState => schema_for!(browser_use_dom::BrowserStateSummary),
            };
            println!("{}", serde_json::to_string_pretty(&schema)?);
        }
        None if cli.version_target => {
            println!("{}", browser_use_core::INITIAL_UPSTREAM_COMMIT);
        }
        None => {}
    }

    Ok(())
}
