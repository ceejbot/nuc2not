use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use notion_client::objects::page::PageProperty;
use notion_client::objects::rich_text::{RichText, Text};
use nuclino_rs::{Collection, Item, Page};

pub fn properties_from_nuclino(page: &Page) -> BTreeMap<String, PageProperty> {
    let mut properties: BTreeMap<String, PageProperty> = BTreeMap::new();

    properties.insert(
        "title".to_string(),
        PageProperty::Title {
            id: None,
            title: vec![simple_rich_text(page.title())],
        },
    );

    let created_time: DateTime<Utc> = page.created().parse().unwrap_or_else(|_| Utc::now());
    properties.insert(
        "created_time".to_string(),
        PageProperty::CreatedTime { id: None, created_time },
    );
    let last_edited_time = match page.modified().parse::<DateTime<Utc>>() {
        Ok(v) => Some(v),
        Err(_) => None,
    };
    properties.insert(
        "last_edited_time".to_string(),
        PageProperty::LastEditedTime {
            id: None,
            last_edited_time,
        },
    );

    // Now here's where we have to treat collections differently from pages.
    match page {
        Page::Item(v) => add_item_props(v, properties),
        Page::Collection(v) => add_collection_props(v, properties),
    }
}

fn add_item_props(item: &Item, properties: BTreeMap<String, PageProperty>) -> BTreeMap<String, PageProperty> {
    if let Some(_content) = item.content() {

        // todo
        // pull in the other module and convert the markdown to properties
    }

    properties
}

fn add_collection_props(
    collection: &Collection,
    properties: BTreeMap<String, PageProperty>,
) -> BTreeMap<String, PageProperty> {
    collection.children().iter().for_each(|_id| {

        // look up url for each link
        // insert link blocktype for each
        // insert links into properties
    });

    properties
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
