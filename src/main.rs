//! Documentation comment here please.

#![deny(future_incompatible, clippy::unwrap_used)]
#![warn(rust_2018_idioms, trivial_casts)]

mod notion;
mod workspace;

use std::process::exit;

use clap::Parser;
use fzf_wrapped::{run_with_output, Fzf};
use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use workspace::WorkspaceCache;

#[derive(Parser, Debug)]
#[clap(name = "nuclino-to-notion", version)]
pub struct Args {
    /// Populate the cache for the chosen Nuclino workspace.
    #[clap(long, short, global = true)]
    populate: bool,
    /// How many milliseconds to wait between Nuclino requests.
    #[clap(long, short, global = true, default_value = "500")]
    wait: usize,
    /// An optional parent page for the imported items. If not provided, the tool won't
    /// try migrate pages to Notion.
    parent: Option<String>,
}

/// Process command-line options and act on them.
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let _ignored = dotenvy::dotenv().into_diagnostic()?;
    let notion_key = std::env::var("NOTION_API_KEY")
        .expect("You must provide a Notion api key in the env var NOTION_API_KEY.");
    let nuclino_key = std::env::var("NUCLINO_API_KEY")
        .expect("You must provide a Nuclino api key in the env var NUCLINO_API_KEY.");

    let client = nuclino_rs::Client::create(nuclino_key.as_str(), None);
    let workspaces = client
        .workspace_list(None, None)
        .into_diagnostic()?
        .to_vec();

    let names: Vec<String> = workspaces
        .iter()
        .map(|space| space.name().to_string())
        .collect();
    let fzf = Fzf::builder()
        .border(fzf_wrapped::Border::Rounded)
        .border_label("Select a workspace to migrate")
        .build()
        .into_diagnostic()?;
    let Some(to_migrate) = run_with_output(fzf, names) else {
        println!("Nothing to do.");
        exit(0);
    };

    let Some(found) = workspaces
        .iter()
        .find(|space| space.name() == to_migrate.as_str())
    else {
        println!("No workspace of that name exists, to everyone's surprise.");
        exit(1);
    };

    println!("Migrating the {} workspace...", to_migrate.blue());
    let cache = WorkspaceCache::new(found, nuclino_key, notion_key, &args)?;
    if args.populate {
        println!("Populating the Nuclino cache…");
        cache.populate()?;
    }
    if let Some(_parent) = args.parent {
        println!("Doing migration…");
        cache.migrate().await?;
    }

    Ok(())
}
