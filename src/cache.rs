//! A cache for a Nuclino instance.

use std::collections::HashSet;
use std::fmt::Display;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use miette::{miette, Context, IntoDiagnostic, Result};
use nuclino_rs::{File, Item, Page, User, Uuid, Workspace};
use once_cell::sync::Lazy;
use owo_colors::OwoColorize;
use serde::{Deserialize, Serialize};
use slug::slugify;

use crate::Args;

static WAIT_UNTIL: Lazy<Mutex<Instant>> = Lazy::new(|| Mutex::new(Instant::now()));

static CACHE_BASE: &str = ".cache";

#[derive(Debug)]
pub struct Cache {
    root: String,
    nuclino: nuclino_rs::Client,
    min_delay: u64, // not usize
    cached: HashSet<Uuid>,
    pending: HashSet<Uuid>,
    workspace: Workspace,
}

impl Cache {
    pub fn new(apikey: String, args: &Args, of_interest: &Workspace) -> Result<Self> {
        let nuclino = nuclino_rs::Client::create(apikey.as_str(), None);
        let name = std::env::var("CACHE_NAME").unwrap_or("generic".to_string());
        let pending = HashSet::new();
        let workspace = of_interest.clone();

        let root = format!("{CACHE_BASE}/{}/{}", slugify(name.clone()), slugify(workspace.name()));
        std::fs::create_dir_all(root.as_str())
            .into_diagnostic()
            .context("Creating cache directory for workspace")?;
        let idset: HashSet<Uuid> = std::fs::read_dir(root.as_str())
            .into_diagnostic()?
            .filter_map(|xs| match xs {
                Ok(fname) => match fname.file_name().to_string_lossy().split('_').last() {
                    Some(idstr) => match idstr.split('.').next() {
                        Some(base) => Uuid::try_from(base).ok(),
                        None => None,
                    },
                    None => None,
                },
                Err(_) => None,
            })
            .collect();
        println!("found {} items in cache for workspace", idset.len());

        Ok(Self {
            root,
            nuclino,
            min_delay: args.wait,
            cached: idset,
            pending,
            workspace: workspace.clone(),
        })
    }

    pub fn cache_workspace(&mut self) -> Result<usize> {
        let oh_no = self.workspace.clone();
        self.save_item(&oh_no, oh_no.id()).context("saving workspace")?;
        let _cached: Result<Vec<Page>, _> = oh_no.children().iter().map(|id| self.cache_page(id)).collect();
        Ok(self.cached.len())
    }

    fn file_path(&self, slug: &str, id: impl Display) -> String {
        format!("{}/{slug}_{id}", self.root)
    }

    pub fn load_item<T>(&self, id: &Uuid) -> Result<T>
    where
        T: Cacheable + Fetchable,
    {
        let fpath = format!("{}.json", self.file_path(T::slug(), id));
        T::load(fpath.as_str()).map(|xs| *xs)
    }

    fn fetch_item<T>(&self, id: &Uuid, refresh: bool) -> Result<T>
    where
        T: Fetchable + Cacheable,
    {
        if !refresh && self.cached.contains(id) {
            self.load_item(id)
        } else {
            self.do_delay();
            println!("    fetching {} id={}", T::slug().blue(), id.yellow());
            T::fetch(&self.nuclino, id).map(|xs| *xs)
        }
    }

    /// Doing our delay between requests to Nuclino to deal with their rate limiting.
    fn do_delay(&self) {
        let mut when = WAIT_UNTIL.lock().expect("well, that was surprising");
        let now = Instant::now();
        if now < *when {
            let delta = *when - now;
            std::thread::sleep(delta);
        }
        *when = Instant::now() + Duration::from_millis(self.min_delay);
    }

    fn save_item<T>(&mut self, item: &T, id: &Uuid) -> Result<()>
    where
        T: Fetchable + Cacheable,
    {
        if !self.cached.contains(id) {
            let fpath = format!("{}.json", self.file_path(T::slug(), id));
            item.save(fpath.clone()).context(format!("saving {fpath}"))?;
            self.cached.insert(*id);
            self.pending.remove(id); // okay if it's not there
        }
        Ok(())
    }

    pub fn cache_page(&mut self, id: &Uuid) -> Result<Page> {
        if self.pending.contains(id) {
            return Err(miette!("Declining to fetch a page twice"));
        }
        let page = self.fetch_item::<Page>(id, false)?;
        println!("        got '{}'", page.title().blue());
        self.pending.insert(*id);

        if let Ok(creator) = self.fetch_item::<User>(page.created_by(), false) {
            self.save_item(&creator, creator.id())?;
        }

        if let Ok(modifier) = self.fetch_item::<User>(page.modified_by(), false) {
            self.save_item(&modifier, modifier.id())?;
        }

        match page {
            Page::Item(ref item) => {
                // items have content_meta
                let _ignored = self.cache_meta(item); // for now
            }
            Page::Collection(ref collection) => {
                // collections have children
                collection.children().iter().for_each(|subpage| {
                    let _ignored = self.cache_page(subpage); // for now
                });
            }
        }
        match self.save_item(&page, page.id()) {
            Ok(_) => {}
            Err(e) => {
                println!("    {} save failed: {e:?}", page.title().blue());
            }
        }

        Ok(page)
    }

    fn cache_meta(&mut self, item: &Item) -> Result<()> {
        println!(
            "        + mentioned pages; count={}",
            item.content_meta().item_ids.len()
        );
        item.content_meta().item_ids.iter().for_each(|id| {
            let _ignored = self.cache_page(id); // for now
        });

        println!("        + attached files; count={}", item.content_meta().file_ids.len());
        item.content_meta().file_ids.iter().for_each(|id| {
            if let Err(e) = self.cache_file(id) {
                eprintln!("{e:?}");
            }
        });

        Ok(())
    }

    fn cache_file(&mut self, id: &Uuid) -> Result<()> {
        let file_info = self.fetch_item::<File>(id, false).context("load file info from disk")?;

        let fpath = self.file_path(File::slug(), file_info.filename());
        if std::path::PathBuf::from(fpath).exists() {
            return Ok(());
        }

        let file_info = self
            .fetch_item::<File>(id, true)
            .context("fetching file info from network")?;
        self.save_item(&file_info, file_info.id())?;
        let dlurl = file_info.download_info().url.clone();
        // println!("            downloading file data {}", file_info.filename().blue());
        let bytes = self.nuclino.download_file(dlurl.as_str()).into_diagnostic()?;

        let fpath = self.file_path(File::slug(), file_info.filename());
        println!("            {}; data length={}", fpath.blue(), bytes.len());
        std::fs::write(fpath, bytes).into_diagnostic()?;

        Ok(())
    }

    pub fn _load_file(&self, file_info: &File) -> Result<Vec<u8>> {
        let fpath = self.file_path(File::slug(), file_info.filename());
        println!("file path is {}", fpath.blue());
        let bytes = std::fs::read(fpath)
            .into_diagnostic()
            .context("loading file path {fpath}")?;
        Ok(bytes)
    }
}

pub trait Cacheable {
    fn load(fpath: &str) -> Result<Box<Self>>;
    fn save(&self, fpath: String) -> Result<()>;
}

impl<T> Cacheable for T
where
    T: for<'de> Deserialize<'de> + Serialize + Clone,
{
    /// Load the data from a local cache file and deserialize.
    fn load(fpath: &str) -> Result<Box<Self>> {
        let bytes = std::fs::read(fpath).into_diagnostic()?;
        let data = serde_json::from_slice::<T>(bytes.as_slice()).into_diagnostic()?;
        Ok(Box::new(data))
    }

    /// Serialize the data to a file in the local cache.
    fn save(&self, fpath: String) -> Result<()> {
        let bytes = serde_json::to_vec(self).into_diagnostic()?;
        std::fs::write(fpath, bytes).into_diagnostic()?;
        Ok(())
    }
}

pub trait Fetchable {
    /// Prepended to file names. This exists to make the files accessible to humans, at least a little.
    fn slug() -> &'static str;
    /// Fetch the data from Nuclino.
    fn fetch(nuclino: &nuclino_rs::Client, id: &Uuid) -> Result<Box<Self>>;
}

impl Fetchable for Page {
    fn slug() -> &'static str {
        "page"
    }

    fn fetch(nuclino: &nuclino_rs::Client, id: &Uuid) -> Result<Box<Self>> {
        nuclino.page(id).map(Box::new).into_diagnostic()
    }
}

impl Fetchable for User {
    fn slug() -> &'static str {
        "user"
    }

    fn fetch(nuclino: &nuclino_rs::Client, id: &Uuid) -> Result<Box<Self>> {
        nuclino.user(id).map(Box::new).into_diagnostic()
    }
}

impl Fetchable for File {
    fn slug() -> &'static str {
        "file"
    }

    fn fetch(nuclino: &nuclino_rs::Client, id: &Uuid) -> Result<Box<Self>> {
        nuclino.file(id).map(Box::new).into_diagnostic()
    }
}

impl Fetchable for Workspace {
    fn slug() -> &'static str {
        "workspace"
    }

    fn fetch(nuclino: &nuclino_rs::Client, id: &Uuid) -> Result<Box<Self>> {
        nuclino.workspace(id).map(Box::new).into_diagnostic()
    }
}
