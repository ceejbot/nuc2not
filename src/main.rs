//! Documentation comment here please.

#![deny(future_incompatible, clippy::unwrap_used)]
#![warn(rust_2018_idioms, trivial_casts)]

mod cache;
mod migrator;

use std::process::exit;

use cache::Cache;
use clap::{Parser, Subcommand};
use fzf_wrapped::{run_with_output, Fzf};
use miette::{IntoDiagnostic, Result};
use nuclino_rs::Workspace;
use owo_colors::OwoColorize;

#[derive(Parser, Debug)]
#[clap(name = "nuclino-to-notion", version)]
pub struct Args {
    /// How many milliseconds to wait between Nuclino requests.
    #[clap(long, short, global = true, default_value = "750")]
    wait: u64,
    #[clap(subcommand)]
    cmd: Command,
}

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Cache a Nuclino workspace locally. You'll be prompted to select the workspace.
    Cache,
    /// Inspect your local cache, listing pages by id.
    InspectCache,
    /// Migrate a single page by id. If the page has media, you'll be prompted to
    /// upload the media by hand: the Notion API does not have endpoints for doing
    /// this automatically.
    MigratePage {
        /// The id of the Nuclino page to migrate
        page: String,
        /// The id of the Notion page to migrate to.
        parent: String,
    },
    /// Migrate a previously-cached Nuclino workspace to Notion. Unreliable!!
    MigrateWorkspace {
        /// A parent Notion page for the migrated items.
        parent: String,
    },
}

fn choose_workspace(nuclino_key: &str) -> Result<Workspace> {
    let client = nuclino_rs::Client::create(nuclino_key, None);
    let workspaces = client.workspace_list(None, None).into_diagnostic()?.to_vec();

    let mut names: Vec<String> = workspaces.iter().map(|space| space.name().to_string()).collect();
    names.sort();
    let fzf = Fzf::builder()
        .border(fzf_wrapped::Border::Rounded)
        .border_label("Select a workspace to migrate")
        .build()
        .into_diagnostic()?;
    let Some(to_migrate) = run_with_output(fzf, names) else {
        println!("Nothing to do.");
        exit(0);
    };

    let Some(found) = workspaces.into_iter().find(|space| space.name() == to_migrate.as_str()) else {
        println!("No workspace of that name exists, to everyone's surprise.");
        exit(1);
    };

    Ok(found)
}

/// Process command-line options and act on them.
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let _ignored = dotenvy::dotenv().into_diagnostic()?;
    let notion_key =
        std::env::var("NOTION_API_KEY").expect("You must provide a Notion api key in the env var NOTION_API_KEY.");
    let nuclino_key =
        std::env::var("NUCLINO_API_KEY").expect("You must provide a Nuclino api key in the env var NUCLINO_API_KEY.");

    let found = choose_workspace(nuclino_key.as_str())?;
    let mut cache = Cache::new(nuclino_key, &args, &found)?;

    match args.cmd {
        Command::Cache => {
            println!("Caching the {} workspace...", found.name().blue());
            let count = cache.cache_workspace()?;
            println!("    {count} items cached");
        }
        Command::InspectCache => {
            cache.print_details()?;
        }
        Command::MigratePage { page, parent } => {
            println!("Migrating page id={}", page.bold());
            let migrator = migrator::Migrator::new(notion_key, parent.clone())?;
            migrator.migrate_one_page(&mut cache, page).await?;
        }
        Command::MigrateWorkspace { parent } => {
            println!("Migrating the {} workspace...", found.name().blue());
            let migrator = migrator::Migrator::new(notion_key, parent)?;
            migrator.migrate(cache, &found).await?;
        }
    }

    Ok(())
}
