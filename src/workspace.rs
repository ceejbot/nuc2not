//! All communication with Nuclino is in this module. We try to fetch a workspace
//! exactly once then hold it on disk. It's the user's responsibility to delete it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Display;
use std::sync::Mutex;

use futures::stream::{self, StreamExt};
use miette::{IntoDiagnostic, Result};
use notion_client::endpoints::blocks::append::request::AppendBlockChildrenRequest;
use notion_client::endpoints::pages::create::request::CreateAPageRequest;
use notion_client::objects::block::{Block, BlockType, BulletedListItemValue, TextColor};
use notion_client::objects::page::{Page as NotionPage, PageProperty};
use notion_client::objects::parent::Parent;
use notion_client::objects::rich_text::{self, Annotations, RichText};
use notion_client::objects::user::User as NotionUser;
use nuclino_rs::{Collection, File, Item, Page, User, Uuid, Workspace};
use once_cell::sync::Lazy;
use owo_colors::OwoColorize;
use serde::Deserialize;
use slug::slugify;

use crate::Args;

static CACHE_BASE: &str = "./nuclino_cache";

static SEEN_PAGES: Lazy<Mutex<HashSet<Uuid>>> = Lazy::new(|| Mutex::new(HashSet::new()));
static SEEN_USERS: Lazy<Mutex<HashSet<Uuid>>> = Lazy::new(|| Mutex::new(HashSet::new()));
static URL_MAP: Lazy<Mutex<HashMap<String, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn pages() -> std::sync::MutexGuard<'static, HashSet<Uuid>> {
    SEEN_PAGES
        .lock()
        .expect("Unrecoverable runtime problem: cannot acquire pages hashset lock. Exiting.")
}

pub fn users() -> std::sync::MutexGuard<'static, HashSet<Uuid>> {
    SEEN_USERS
        .lock()
        .expect("Unrecoverable runtime problem: cannot acquire users hashset lock. Exiting.")
}

pub fn urlmap() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
    URL_MAP
        .lock()
        .expect("Unrecoverable runtime problem: cannot acquire pages hashset lock. Exiting.")
}

pub enum DataKind {
    File,
    FileInfo,
    Page,
    User,
    Workspace,
}

impl Display for DataKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataKind::File => write!(f, "file"),
            DataKind::FileInfo => write!(f, "file_info"),
            DataKind::Page => write!(f, "page"),
            DataKind::User => write!(f, "user"),
            DataKind::Workspace => write!(f, "workspace"),
        }
    }
}

pub trait FromNuclino {
    fn fetch(nuclino: &nuclino_rs::Client, id: &Uuid) -> Result<Box<Self>>;
    fn datakind() -> DataKind;
}

pub trait Cacheable {
    fn load(fpath: &str) -> Result<Box<Self>>;
    fn load_or_fetch(nuclino: &nuclino_rs::Client, fpath: &str, id: &Uuid) -> Result<Box<Self>>;
}

impl<T> Cacheable for T
where
    T: for<'de> Deserialize<'de> + Clone + FromNuclino,
{
    fn load_or_fetch(nuclino: &nuclino_rs::Client, fpath: &str, id: &Uuid) -> Result<Box<Self>> {
        if let Ok(found) = Self::load(fpath) {
            Ok(found)
        } else {
            let item = Self::fetch(nuclino, id)?;
            Ok(item)
        }
    }

    fn load(fpath: &str) -> Result<Box<Self>> {
        let bytes = std::fs::read(fpath).into_diagnostic()?;
        let data = serde_json::from_slice::<T>(bytes.as_slice()).into_diagnostic()?;
        Ok(Box::new(data))
    }
}

impl FromNuclino for nuclino_rs::User {
    fn fetch(nuclino: &nuclino_rs::Client, id: &Uuid) -> Result<Box<Self>> {
        match nuclino.user(id) {
            Ok(user) => Ok(Box::new(user)),
            Err(e) => Err(e).into_diagnostic(),
        }
    }

    fn datakind() -> DataKind {
        DataKind::User
    }
}

impl FromNuclino for Page {
    fn fetch(nuclino: &nuclino_rs::Client, id: &Uuid) -> Result<Box<Self>> {
        match nuclino.page(id) {
            Ok(page) => Ok(Box::new(page)),
            Err(e) => Err(e).into_diagnostic(),
        }
    }

    fn datakind() -> DataKind {
        DataKind::Page
    }
}

impl FromNuclino for nuclino_rs::File {
    fn fetch(nuclino: &nuclino_rs::Client, id: &Uuid) -> Result<Box<Self>> {
        match nuclino.file(id) {
            Ok(file_info) => Ok(Box::new(file_info)),
            Err(e) => Err(e).into_diagnostic(),
        }
    }

    fn datakind() -> DataKind {
        DataKind::FileInfo
    }
}

impl FromNuclino for nuclino_rs::Workspace {
    fn fetch(nuclino: &nuclino_rs::Client, id: &Uuid) -> Result<Box<Self>> {
        match nuclino.workspace(id) {
            Ok(ws) => Ok(Box::new(ws)),
            Err(e) => Err(e).into_diagnostic(),
        }
    }

    fn datakind() -> DataKind {
        DataKind::Workspace
    }
}

// maybe: a pointer to where the cache is stored, and the root node to start walking from
pub struct WorkspaceCache {
    workspace: Workspace,
    slug: String,
    nuclino: nuclino_rs::Client,
    notion: notion_client::endpoints::Client,
    parent: String,
}

impl WorkspaceCache {
    pub fn new(space: &Workspace, nuclino_key: String, notion_key: String, args: &Args) -> Result<Self> {
        println!("Workspace {} has {} children", space.name(), space.children().len());
        let nuclino = nuclino_rs::Client::create(nuclino_key.as_str(), None);
        let notion = notion_client::endpoints::Client::new(notion_key, None).into_diagnostic()?;
        Ok(Self {
            workspace: space.clone(),
            slug: slugify(space.name()),
            nuclino,
            notion,
            parent: args.parent.clone().unwrap_or_default(),
        })
    }

    pub fn populate(&self) -> Result<()> {
        if let Err(e) = std::fs::create_dir_all(format!("{CACHE_BASE}/{}", self.slug)) {
            if !matches!(e.kind(), std::io::ErrorKind::AlreadyExists) {
                return Err(e).into_diagnostic();
            }
        }
        if let Err(e) = std::fs::create_dir_all(format!("{CACHE_BASE}/users")) {
            if !matches!(e.kind(), std::io::ErrorKind::AlreadyExists) {
                return Err(e).into_diagnostic();
            }
        }
        let stringified = serde_json::to_string(&self.workspace).into_diagnostic()?;
        let fpath = self.file_path(DataKind::Workspace, self.workspace.id());
        std::fs::write(fpath, stringified.as_bytes()).into_diagnostic()?;

        let _cached: Result<Vec<Page>, _> = self.workspace.children().iter().map(|id| self.cache_page(id)).collect();

        println!(
            "Cached {} pages and {} users.",
            pages().len().blue(),
            users().len().blue()
        );

        Ok(())
    }

    pub fn file_path(&self, kind: DataKind, stringy: impl Display) -> String {
        match kind {
            DataKind::File => format!("{CACHE_BASE}/{}/{stringy}", self.slug),
            DataKind::FileInfo => format!("{CACHE_BASE}/{}/{kind}_{stringy}", self.slug),
            DataKind::Page => format!("{CACHE_BASE}/{}/{kind}_{stringy}", self.slug),
            DataKind::User => format!("{CACHE_BASE}/users/{stringy}"),
            DataKind::Workspace => format!("{CACHE_BASE}/{}/{kind}_{stringy}", self.slug),
        }
    }

    fn load_or_fetch<T: FromNuclino + Cacheable>(&self, id: &Uuid) -> Result<T> {
        let fpath = self.file_path(T::datakind(), id);
        let data = T::load_or_fetch(&self.nuclino, fpath.as_str(), id)?;
        Ok(*data)
    }

    fn load_item<T: FromNuclino + Cacheable>(&self, id: &Uuid) -> Result<T> {
        let fpath = self.file_path(T::datakind(), id);
        let data = T::load(fpath.as_str());
        data.map(|xs| *xs)
    }

    fn cache_page(&self, id: &Uuid) -> Result<Page> {
        if pages().contains(id) {
            return self.load_item::<Page>(id);
        }

        let page = self.load_or_fetch::<Page>(id)?;
        self.cache_user(page.created_by())?;
        self.cache_user(page.modified_by())?;

        match page {
            Page::Item(ref item) => {
                // items have content_meta
                self.cache_meta(item)?;
            }
            Page::Collection(ref collection) => {
                // collections have children
                collection.children().iter().for_each(|id| {
                    if let Err(e) = self.cache_page(id) {
                        println!("err: {e:?}");
                    }
                });
            }
        }
        pages().insert(*id);
        Ok(page)
    }

    fn cache_meta(&self, item: &Item) -> Result<()> {
        item.content_meta().item_ids.iter().for_each(|id| {
            if let Err(e) = self.cache_page(id) {
                println!("err: {e:?}");
            }
        });

        item.content_meta().file_ids.iter().for_each(|id| {
            let _ignored = self.cache_file(id);
        });

        Ok(())
    }

    fn cache_file(&self, id: &Uuid) -> Result<()> {
        let file_info = self.load_or_fetch::<File>(id)?;
        let dlurl = file_info.download_info().url.clone();
        let bytes = self.nuclino.download_file(dlurl.as_str()).into_diagnostic()?;

        let fpath = self.file_path(DataKind::File, file_info.filename());
        std::fs::write(fpath, bytes).into_diagnostic()?;

        Ok(())
    }

    fn cache_user(&self, id: &Uuid) -> Result<()> {
        if users().contains(id) {
            return Ok(());
        }

        let _user = self.load_or_fetch::<User>(id)?;
        users().insert(*id);
        Ok(())
    }

    // now push all the pages into Notion...

    pub async fn migrate(&self) -> Result<()> {
        let futures: Vec<_> = self
            .workspace
            .children()
            .iter()
            .map(|id| async { self.migrate_page(id, self.parent.as_str()).await })
            .collect();
        let mut buffered = stream::iter(futures).buffer_unordered(3);
        while let Some(child_result) = buffered.next().await {
            let child = child_result?;
            println!("migrated {:?}", child);
            // println!("{:?}", urlmap());
        }

        // emit some info

        Ok(())
    }

    async fn migrate_page(&self, id: &Uuid, parent_id: &str) -> Result<NotionPage> {
        let page = self.load_item::<Page>(id)?;

        // Create empty page: no content, no children.
        // Insert new url into url map.
        // Call migrate_page all all children recursively.
        // When done, rewrite content with urls and update the page object in Notion.

        let mut properties = crate::notion::properties_from_nuclino(&page);

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

        let migrated = match page {
            Page::Item(item) => self.migrate_item(&item, parent_id, properties).await?,
            Page::Collection(collection) => self.migrate_collection(&collection, parent_id, properties).await?,
        };
        Ok(migrated)
    }

    async fn migrate_item(
        &self,
        item: &Item,
        parent_id: &str,
        properties: BTreeMap<String, PageProperty>,
    ) -> Result<NotionPage> {
        let parent = Parent::PageId {
            page_id: parent_id.to_string(),
        };
        let children = item.content().map(|content| md2notion::convert(content.as_str()));
        let new_page_req = CreateAPageRequest {
            parent,
            icon: None,
            cover: None,
            properties,
            children,
        };
        let notion_page = self.notion.pages.create_a_page(new_page_req).await.into_diagnostic()?;
        urlmap().insert(item.url().to_string(), notion_page.url.clone());

        let _meta = item.content_meta();
        // deal with item_ids
        // deal with file_ids

        todo!()
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

    async fn look_up_user(&self, _nuclino_id: &Uuid) -> Option<NotionUser> {
        todo!()
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
