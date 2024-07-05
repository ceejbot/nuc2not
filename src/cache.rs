//! A cache for a Nuclino instance.

use std::collections::HashSet;
use std::fmt::Display;

use miette::{IntoDiagnostic, Result};
use nuclino_rs::{File, Item, Page, User, Uuid, Workspace};
use serde::{Deserialize, Serialize};
use slug::slugify;

use crate::Args;

static CACHE_BASE: &str = ".cache";

pub struct Cache {
    root: String,
    name: String,
    apikey: String,
    nuclino: nuclino_rs::Client,
    min_delay: usize,
    cached: HashSet<Uuid>,
}

impl Cache {
    pub fn new(name: String, apikey: String, args: &Args) -> Result<Self> {
        let nuclino = nuclino_rs::Client::create(apikey.as_str(), None);
        let root = format!("{CACHE_BASE}/{}", slugify(name.clone()));
        let rootstr = root.as_str();
        let metadata = std::fs::metadata(rootstr).into_diagnostic()?;
        let cached = if metadata.is_dir() {
            let idset: HashSet<Uuid> = std::fs::read_dir(rootstr)
                .into_diagnostic()?
                .filter_map(|xs| {
                    if let Ok(fname) = xs {
                        if fname.file_name().is_ascii() {
                            if let Some(idstr) = fname.file_name().to_string_lossy().split('_').last() {
                                if let Ok(id) = Uuid::try_from(idstr) {
                                    return Some(id);
                                } else {
                                    return None;
                                }
                            } else {
                                return None;
                            }
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                })
                .collect();
            idset
        } else {
            std::fs::create_dir_all(rootstr).into_diagnostic()?;
            HashSet::new()
        };

        Ok(Self {
            root,
            name,
            apikey,
            nuclino,
            min_delay: args.wait,
            cached,
        })
    }

    pub fn cache_workspace(&mut self, workspace: &Workspace) -> Result<usize> {
        self.save_item(workspace, workspace.id())?;
        let cached: Result<Vec<Page>, _> = workspace.children().iter().map(|id| self.cache_page(id)).collect();
        Ok(cached?.len())
    }

    fn file_path(&self, slug: &str, id: impl Display) -> String {
        format!("{}/{slug}_{id}", self.root)
    }

    pub fn load_content_map(&mut self) {
        // read all ids from file names in root
        todo!()
    }

    pub fn load_item<T>(&self, id: &Uuid) -> Result<T>
    where
        T: Cacheable + Fetchable,
    {
        let fname = format!("{}/{}_{id}", self.root, T::slug());
        T::load(fname.as_str()).map(|xs| *xs)
    }

    fn fetch_item<T>(&self, id: &Uuid) -> Result<T>
    where
        T: Fetchable,
    {
        // TODO bottleneck for request throttling
        T::fetch(&self.nuclino, id).map(|xs| *xs)
    }

    fn save_item<T>(&mut self, item: &T, id: &Uuid) -> Result<()>
    where
        T: Fetchable + Cacheable,
    {
        item.save(self.file_path(T::slug(), id))?;
        self.cached.insert(*id);
        Ok(())
    }

    fn cache_page(&mut self, id: &Uuid) -> Result<Page> {
        let page = self.fetch_item::<Page>(id)?;

        let creator = self.fetch_item::<User>(page.created_by())?;
        self.save_item(&creator, creator.id())?;

        let modifier = self.fetch_item::<User>(page.modified_by())?;
        self.save_item(&modifier, modifier.id())?;

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
        self.save_item(&page, page.id())?;

        Ok(page)
    }

    fn cache_meta(&mut self, item: &Item) -> Result<()> {
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

    fn cache_file(&mut self, id: &Uuid) -> Result<()> {
        let file_info = self.fetch_item::<File>(id)?;
        let dlurl = file_info.download_info().url.clone();
        let bytes = self.nuclino.download_file(dlurl.as_str()).into_diagnostic()?;

        let fpath = self.file_path(File::slug(), file_info.filename());
        std::fs::write(fpath, bytes).into_diagnostic()?;

        Ok(())
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
