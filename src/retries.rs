//! Wrappers around the Notion client that retry requests that get 409s.
//!

use miette::{IntoDiagnostic, Result};
use notion_client::endpoints::blocks::append::request::AppendBlockChildrenRequest;
use notion_client::endpoints::pages::create::request::CreateAPageRequest;
use notion_client::endpoints::Client;
use notion_client::objects::block::Block;
use notion_client::objects::page::Page as NotionPage;
use owo_colors::OwoColorize;

/// The most we'll retry a 409 conflicted request
static MAX_RETRIES: u8 = 5;

/// Time to delay between requests
static NOTION_DELAY_MS: u64 = 200;

pub async fn do_create(notion: &Client, request: &CreateAPageRequest, retry: u8) -> Result<NotionPage> {
    if retry > 0 {
        println!("    do_create(); retry={}", retry.bold());
    }
    let next_retry = retry + 1;
    match notion.pages.create_a_page(request.clone()).await {
        Ok(resp) => Ok(resp),
        Err(e) => match e {
            notion_client::NotionClientError::InvalidStatusCode { ref error } => {
                if error.status == 409 && retry < MAX_RETRIES {
                    println!("    do_create() got {}; retrying", 409.bold());
                    Box::pin(do_create(notion, request, next_retry)).await
                } else {
                    Err(e).into_diagnostic()
                }
            }
            _ => Err(e).into_diagnostic(),
        },
    }
}

pub async fn do_append(
    notion: &Client,
    parent_id: &str,
    slice: &[Block],
    after: Option<String>,
    retry: u8,
) -> Result<Vec<Block>> {
    if retry > 0 {
        println!("    do_append(); retry={}", retry.bold());
        // println!(
        //     "    doing append; parent_id={parent_id}; after_id={after:?}; children={}; retries: {}",
        //     slice.len(),
        //     retry.bold()
        // );
    }
    let next_retry = retry + 1;
    if slice.is_empty() {
        return Ok(Vec::new());
    }
    let children = slice.to_vec();
    // We're having 409 problems at the speed we're making API requests right now. It is to lol.
    tokio::time::sleep(std::time::Duration::from_millis(NOTION_DELAY_MS)).await;
    let append_req = AppendBlockChildrenRequest {
        children: slice.to_vec(),
        after: after.clone(),
    };
    match notion.blocks.append_block_children(parent_id, append_req).await {
        Ok(response) => Ok(response.results),
        Err(e) => match e {
            notion_client::NotionClientError::InvalidStatusCode { ref error } => {
                if error.status == 409 && retry < MAX_RETRIES {
                    println!("    do_append() got {}; retrying", 409.bold());
                    Box::pin(do_append(notion, parent_id, children.as_slice(), after, next_retry)).await
                } else {
                    Err(e).into_diagnostic()
                }
            }
            _ => Err(e).into_diagnostic(),
        },
    }
}
