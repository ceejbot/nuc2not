//! Parse a single markdown file and make a page from it.
//! As a shortcut, reads a Notion secret key from the environment
//! and a destination parent page id as well.

use std::collections::BTreeMap;

use md2notion::convert;
use notion_client::endpoints::pages::create::request::CreateAPageRequest;
use notion_client::objects::page::PageProperty;
use notion_client::objects::parent::Parent;
use notion_client::objects::rich_text::{RichText, Text};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ignored = dotenvy::dotenv()?;
    let notion_key = std::env::var("NOTION_API_KEY")
        .expect("You must provide a Notion api key in the env var NOTION_API_KEY.");
    let parent_id = std::env::var("NOTION_PARENT")
        .expect("You must provide a Notion parent page id in the env var NOTION_PARENT.");

    let client = notion_client::endpoints::Client::new(notion_key, None)?;

    let input = include_str!("../fixtures/table.md");
    let blocks = convert(input);

    // The Notion API structures are quite baroque.
    let parent = Parent::PageId {
        page_id: parent_id.to_string(),
    };
    let mut properties: BTreeMap<String, PageProperty> = BTreeMap::new();
    properties.insert(
        "title".to_string(),
        PageProperty::Title {
            id: None,
            title: vec![simple_rich_text("Example markdown conversion")],
        },
    );

    let new_page_req = CreateAPageRequest {
        parent,
        icon: None,
        cover: None,
        properties,
        children: Some(blocks),
    };
    let _notion_page = client.pages.create_a_page(new_page_req).await?;

    Ok(())
}

pub fn simple_rich_text(input: &str) -> RichText {
    let text = Text {
        content: input.to_string(),
        link: None,
    };
    RichText::Text {
        text,
        annotations: None,
        plain_text: None,
        href: None,
    }
}
