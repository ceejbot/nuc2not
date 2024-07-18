//! Migrator.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use futures::stream::{self, StreamExt};
use miette::{miette, IntoDiagnostic, Result};
use notion_client::endpoints::blocks::append::request::AppendBlockChildrenRequest;
use notion_client::endpoints::pages::create::request::CreateAPageRequest;
use notion_client::endpoints::Client;
use notion_client::objects::block::{Block, BlockType, BulletedListItemValue, TextColor};
use notion_client::objects::page::{Page as NotionPage, PageProperty};
use notion_client::objects::parent::Parent;
use notion_client::objects::rich_text::{self, Annotations, RichText, Text};
use nuclino_rs::{Collection, Item, Page, Uuid, Workspace};
use once_cell::sync::{Lazy, OnceCell};
use owo_colors::OwoColorize;

use crate::Cache;
use nuc2not::create_page;

static URL_MAP: Lazy<Mutex<HashMap<String, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn urlmap() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
    URL_MAP
        .lock()
        .expect("Unrecoverable runtime problem: cannot acquire pages hashset lock. Exiting.")
}

static CACHE: OnceCell<Cache> = OnceCell::new();

fn cache() -> &'static Cache {
    CACHE
        .get()
        .expect("runtime error: migrator cannot access its cache object; exiting")
}

#[derive(Debug, Clone)]
pub struct Migrator {
    notion: Client,
    parent: String,
}

impl Migrator {
    pub fn new(key: String, parent: String) -> Result<Self> {
        let notion = notion_client::endpoints::Client::new(key, None).into_diagnostic()?;

        Ok(Self { notion, parent })
    }

    /// We walk workspace children instead of getting a full list of workspace pages
    /// so that we can guarantee that any links on a specific page have been migrated
    /// and have Notion URLs before we try to migrate the page itself.
    pub async fn migrate(&self, cache: Cache, workspace: &Workspace) -> Result<()> {
        let _ignored = CACHE.set(cache);

        // Is there a better way?
        let futures: Vec<_> = workspace
            .children()
            .iter()
            .map(|id| async { self.migrate_page(id, self.parent.as_str()).await })
            .collect();
        let mut buffered = stream::iter(futures).buffer_unordered(3);
        while let Some(child_result) = buffered.next().await {
            let child = child_result?;
            println!("migrated {:?}", child);
        }

        // emit some info
        // println!("{:?}", urlmap());

        Ok(())
    }

    async fn migrate_page(&self, id: &Uuid, parent: &str) -> Result<NotionPage> {
        let page = cache().load_item::<Page>(id)?;
        eprintln!("Migrating page {}…", page.title().bold().green());

        let properties = properties_from_nuclino(&page);

        /*
        if let Some(creator) = self.look_up_user(page.created_by()).await {
            properties.insert(
                "created_by".to_string(),
                PageProperty::CreatedBy {
                    id: None,
                    created_by: creator,
                },
            );
        }

        if let Some(modifier) = self.look_up_user(page.modified_by()).await {
            properties.insert(
                "edited_by".to_string(),
                PageProperty::LastEditedBy {
                    id: None,
                    last_edited_by: modifier,
                },
            );
        }
        */

        // Now we migrate the content for this item, because the url map will now
        // let us rewrite the urls.
        let migrated = match page {
            Page::Item(item) => self.migrate_item(&item, parent, properties).await?,
            Page::Collection(collection) => self.migrate_collection(&collection, parent, properties).await?,
        };
        println!("    page migrated.");
        Ok(migrated)
    }

    async fn migrate_item(
        &self,
        item: &Item,
        parent_id: &str,
        properties: BTreeMap<String, PageProperty>,
    ) -> Result<NotionPage> {
        if let Some(content) = item.content() {
            let remapped = self.remap(content);
            let notion_page = create_page(&self.notion, remapped.as_str(), parent_id, properties).await?;
            urlmap().insert(item.url().to_string(), notion_page.url.clone());
            let _meta = item.content_meta();
            // TODO deal with item_ids
            // TODO deal with file_ids

            Ok(notion_page)
        } else {
            Err(miette!("page had no content; skipping"))
        }
    }

    /// Rewrite any urls to nuclino content to their new nuclino homes.
    fn remap(&self, input: &str) -> String {
        // TODO This is insufficient
        urlmap()
            .iter()
            .fold(input.to_owned(), |current, (nuc, not)| current.replace(nuc, not))
    }

    async fn migrate_collection(
        &self,
        collection: &Collection,
        parent_id: &str,
        properties: BTreeMap<String, PageProperty>,
    ) -> Result<NotionPage> {
        let parent = Parent::PageId {
            page_id: parent_id.to_string(),
        };
        let new_page_req = CreateAPageRequest {
            parent,
            icon: None,
            cover: None,
            properties,
            children: None,
        };
        let notion_page = self.notion.pages.create_a_page(new_page_req).await.into_diagnostic()?;
        urlmap().insert(collection.url().to_string(), notion_page.url.clone());

        let mut subpages: Vec<NotionPage> = Vec::new();
        let futures: Vec<_> = collection
            .children()
            .iter()
            .map(|child_id| async { self.migrate_page(child_id, notion_page.id.as_str()).await })
            .collect();
        let mut buffered = stream::iter(futures).buffer_unordered(3);
        while let Some(child_result) = buffered.next().await {
            let child = child_result?;
            subpages.push(child);
        }

        // Now we make a bulleted list of links to the sub-pages that looks similar to the Nuclino collection,
        // and update our page with it.
        let blocks: Vec<Block> = subpages.iter().map(make_link_block).collect();
        let req2 = AppendBlockChildrenRequest {
            children: blocks,
            after: None,
        };
        let response = self
            .notion
            .blocks
            .append_block_children(notion_page.id.as_str(), req2)
            .await
            .into_diagnostic()?;
        println!("{response:?}");
        Ok(notion_page.clone())
    }
}

pub fn properties_from_nuclino(page: &Page) -> BTreeMap<String, PageProperty> {
    let mut properties: BTreeMap<String, PageProperty> = BTreeMap::new();

    properties.insert(
        "title".to_string(),
        PageProperty::Title {
            id: None,
            title: vec![simple_rich_text(page.title())],
        },
    );

    /*
    let created_time: DateTime<Utc> = page.created().parse().unwrap_or_else(|_| Utc::now());
    properties.insert(
        "created_time".to_string(),
        PageProperty::CreatedTime { id: None, created_time },
    );
    if let Ok(last_edited_time) = page.modified().parse::<DateTime<Utc>>() {
        properties.insert(
            "last_edited_time".to_string(),
            PageProperty::LastEditedTime {
                id: None,
                last_edited_time: Some(last_edited_time),
            },
        );
    }
    */
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

fn make_link_block(link_to: &NotionPage) -> Block {
    let title = if let Some(PageProperty::Title { title, .. }) = link_to.properties.get("title") {
        title
            .iter()
            .filter_map(|t| t.plain_text())
            .collect::<Vec<String>>()
            .join(" ")
    } else {
        link_to.url.clone()
    };
    let annotations = Annotations::default();
    let page = rich_text::PageMention { id: link_to.id.clone() };
    let mention = rich_text::Mention::Page { page };
    let rich_text = RichText::Mention {
        mention,
        annotations,
        plain_text: title,
        href: Some(link_to.url.clone()),
    };
    let bulleted_list_item = BulletedListItemValue {
        rich_text: vec![rich_text],
        color: TextColor::Default,
        children: None,
    };
    Block {
        block_type: BlockType::BulletedListItem { bulleted_list_item },
        ..Default::default()
    }
}
