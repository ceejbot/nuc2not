//! Migrator.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use futures::stream::{self, StreamExt};
use miette::{miette, IntoDiagnostic, Result};
use notion_client::endpoints::pages::create::request::CreateAPageRequest;
use notion_client::endpoints::Client;
use notion_client::objects::page::{Page as NotionPage, PageProperty};
use notion_client::objects::parent::Parent;
use notion_client::objects::rich_text::{RichText, Text};
use nuc2not::create_page;
use nuclino_rs::{Collection, Item, Page, Uuid, Workspace};
use once_cell::sync::{Lazy, OnceCell};
use owo_colors::OwoColorize;

use crate::Cache;

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
    pub async fn migrate(&self, cachet: Cache, workspace: &Workspace) -> Result<()> {
        self.migrate_pagelist(cachet, workspace.children()).await
    }

    pub async fn migrate_pagelist(&self, cachet: Cache, ids: &[Uuid]) -> Result<()> {
        // a pun with a point. except they're pronounced differently. it is to lol.
        let _ignored = CACHE.set(cachet);
        // Is there a better way?
        let futures: Vec<_> = ids
            .iter()
            .map(|id| async { self.migrate_page(&id.clone(), self.parent.as_str()).await })
            .collect();
        let mut buffered = stream::iter(futures).buffered(2);
        while let Some(child_result) = buffered.next().await {
            if let Err(_e) = child_result {
                // should log it
            }
        }
        Ok(())
    }

    async fn migrate_page(&self, id: &Uuid, parent: &str) -> Result<NotionPage> {
        let page = cache().load_item::<Page>(id)?;
        // eprintln!("    Migrating page {}…", page.title().bold().green());
        let properties = properties_from_nuclino(&page);
        // Now we migrate the content for this item, because the url map will now
        // let us rewrite the urls.
        let migrated = match page {
            Page::Item(ref item) => self.migrate_item(item, parent, properties).await?,
            Page::Collection(ref collection) => self.migrate_collection(collection, parent, properties).await?,
        };
        // println!("    {} migrated.", page.title().bold().green());
        Ok(migrated)
    }

    async fn migrate_item(
        &self,
        item: &Item,
        parent_id: &str,
        properties: BTreeMap<String, PageProperty>,
    ) -> Result<NotionPage> {
        let Some(content) = item.content() else {
            return Err(miette!("page had no content; skipping"));
        };

        let remapped = self.remap(content);
        let notion_page = create_page(&self.notion, remapped.as_str(), parent_id, properties).await?;
        urlmap().insert(item.url().to_string(), notion_page.url.clone());

        let meta = item.content_meta();
        let related_files: Vec<nuclino_rs::File> = meta
            .file_ids
            .iter()
            .filter_map(|xs| cache().load_item::<nuclino_rs::File>(xs).ok())
            .collect();

        println!(
            "        {} migrated to {}",
            item.title().bold().green(),
            notion_page.url.yellow()
        );
        if related_files.is_empty() {
            return Ok(notion_page);
        }

        println!("        To complete the migration, upload each of these files by hand:");
        related_files.iter().for_each(|xs| {
            let fpath = cache().file_path("file", xs.filename()); // erk
            println!("            * {}", fpath.bold());
        });

        /*
                let id = notion_page.id.clone();
                let futures: Vec<_> = infos
                    .iter()
                    .map(|info| async { self.migrate_file(info, &id).await })
                    .collect();
                let mut buffered = stream::iter(futures).buffered(2);
                while let Some(child_result) = buffered.next().await {
                    let _child = child_result?;
                }
        */
        Ok(notion_page)
    }

    async fn _migrate_file(&self, file: &nuclino_rs::File, _parent: &str) -> Result<()> {
        let _bytes = cache()._load_file(file);
        // The API does not support uploading files.
        // record scratch
        Ok(())
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

        let notion_page = nuc2not::do_create(&self.notion, &new_page_req, 0).await?;
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
