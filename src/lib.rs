//! This library exports two reusable functions, one that converts Markdown strings
//! to Notion page content constructs and one that creates Notion pages.

mod retries;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, VecDeque};

use markdown::mdast::{self, Node};
use markdown::{to_mdast, ParseOptions};
use miette::{miette, Result};
use notion_client::endpoints::pages::create::request::CreateAPageRequest;
use notion_client::endpoints::Client;
use notion_client::objects::block::*;
use notion_client::objects::emoji::Emoji;
use notion_client::objects::file::{ExternalFile, File};
use notion_client::objects::page::{Page as NotionPage, PageProperty};
use notion_client::objects::parent::Parent;
use notion_client::objects::rich_text::{Annotations, Equation, Link, RichText, Text};
pub use retries::{do_append, do_create};

/// The deepest level of nesting we'll allow in an API request.
static MAX_NESTING: u8 = 1;

/// Convert a string slice containing Markdown into a Notion Page in your Notion team.
/// This function makes as many API calls as necessary to create the page with
/// all content, working around limits on body size and nesting depth.
pub async fn create_page(
    client: &Client,
    input: &str,
    parent: &str,
    properties: BTreeMap<String, PageProperty>,
) -> Result<NotionPage> {
    let maker = PageMaker::new(client, parent, properties);
    maker.make_page(input).await
}

/// This name amused me, and I wanted to avoid passing a million arguments
/// to some functions.
struct PageMaker {
    notion: Client,
    parent: String,
    properties: BTreeMap<String, PageProperty>,
}

impl PageMaker {
    pub fn new(client: &Client, parent_id: &str, properties: BTreeMap<String, PageProperty>) -> Self {
        PageMaker {
            notion: client.clone(),
            parent: parent_id.to_owned(),
            properties,
        }
    }

    pub async fn make_page(&self, input: &str) -> Result<NotionPage> {
        let blocks = convert(input);
        if blocks.is_empty() {
            // early return for readability
            return Err(miette!("Markdown AST has no children; is the markdown file empty?"));
        }

        let parent = Parent::PageId {
            page_id: self.parent.clone(),
        };
        let new_page_req = CreateAPageRequest {
            parent: parent.clone(),
            icon: None,
            cover: None,
            properties: self.properties.clone(),
            children: None,
        };

        let notion_page = do_create(&self.notion, &new_page_req, 0).await?;

        // Now we have our first ID to hang children on!
        let mut remaining = VecDeque::from(blocks);
        self.append_children(notion_page.id.clone().as_str(), None, &mut remaining)
            .await?;

        Ok(notion_page)
    }

    async fn append_children(
        &self,
        parent_id: &str,
        after_id: Option<String>,
        to_be_appended: &mut VecDeque<Block>,
    ) -> Result<()> {
        eprintln!(
            "entering append_children({parent_id}, {after_id:?}, len={})",
            to_be_appended.len()
        );
        let mut after: Option<String> = after_id.clone();
        let mut current_tranche: Vec<Block> = Vec::new(); // building the next list
        while !to_be_appended.is_empty() {
            if let Some(head) = to_be_appended.pop_front() {
                // While the head of `remaining` has no children, push it onto the end of `blocks`
                // for blocks with children, stop and look to see if the children violate depth limits.
                if PageMaker::block_has_deep_children(0, &head) {
                    // if so, hold that block and call append children on it one level at a time until we hit bottom.
                    // This is not maximally efficient, BUT.
                    let (copy, maybe_children) = split_block_from_children(head);
                    current_tranche.push(copy);
                    let created =
                        do_append(&self.notion, parent_id, current_tranche.as_slice(), after.clone(), 0).await?;
                    // snag the id from the last block in the request, which will be head's id
                    let head_id = if let Some(last) = created.last() {
                        if let Some(ref id) = last.id {
                            id.clone()
                        } else {
                            parent_id.to_owned()
                        }
                    } else {
                        // really quite impossible, which means my structure is wrong here
                        parent_id.to_owned()
                    };
                    if let Some(mut head_children) = maybe_children {
                        Box::pin(self.append_children(head_id.as_str(), None, &mut head_children)).await?;
                    }
                    current_tranche = Vec::new();
                    // keep going with the rest of the list, now with the after-id of where we stopped
                    Box::pin(self.append_children(parent_id, Some(head_id.clone()), to_be_appended)).await?;
                } else {
                    current_tranche.push(head);
                }
                // Magic constant is an API limit. Make the request, then keep on going.
                if current_tranche.len() == 100 {
                    let created =
                        do_append(&self.notion, parent_id, current_tranche.as_slice(), after.clone(), 0).await?;
                    if let Some(last) = created.last() {
                        after.clone_from(&last.id);
                    }
                    current_tranche = Vec::new();
                }
            }
        }

        if !current_tranche.is_empty() {
            let _created = do_append(&self.notion, parent_id, current_tranche.as_slice(), after.clone(), 0).await?;
        }

        Ok(())
    }

    fn block_has_deep_children(nesting: u8, block: &Block) -> bool {
        let maybe_kids = match block.block_type {
            BlockType::BulletedListItem { ref bulleted_list_item } => &bulleted_list_item.children,
            BlockType::NumberedListItem { ref numbered_list_item } => &numbered_list_item.children,
            BlockType::Paragraph { ref paragraph } => &paragraph.children,
            BlockType::Quote { ref quote } => &quote.children,
            _ => &None,
        };
        let Some(children) = maybe_kids else {
            return false;
        };
        if children.is_empty() {
            return false;
        }
        if nesting == MAX_NESTING {
            return true;
        }
        children
            .iter()
            .any(|child| PageMaker::block_has_deep_children(nesting + 1, child))
    }
}

/// Convert a string slice into a vector of Notion blocks. The underpinnings of the page
/// creation function. Unlike that function, this one makes no attempt to work with the
/// API's limitation. It does, however, do its best to represent the Markdown data with
/// Notion block and rich text concepts.
pub fn convert(input: &str) -> Vec<Block> {
    // This function is infallible with the default options.
    let Ok(tree) = to_mdast(input, &ParseOptions::gfm()) else {
        return Vec::new();
    };
    let mut state = State::new();
    state.render(tree)
}

#[derive(Debug, Clone)]
enum ListVariation {
    None,
    Bulleted,
    Ordered,
}

/// We need to track a little state when we're rendering lists, which can be nested.
/// We also need to gather up link and image reference definitions so we can substitute
/// in the full links when we encounter them in the markup.
#[derive(Debug, Clone)]
struct State {
    list: ListVariation,
    ordered_start: u32,
    links: HashMap<String, String>,
    images: HashMap<String, mdast::Image>,
}

impl State {
    pub fn new() -> State {
        State {
            list: ListVariation::None,
            ordered_start: 1,
            links: HashMap::new(),
            images: HashMap::new(),
        }
    }

    /// The function to call to do the work. All of this is infallible.
    pub fn render(&mut self, tree: Node) -> Vec<Block> {
        if let Some(children) = tree.children() {
            self.render_nodes(children)
        } else {
            Vec::new()
        }
    }

    /// Render the passed-in vector of nodes.
    fn render_nodes(&mut self, nodelist: &[Node]) -> Vec<Block> {
        self.collect_definitions(nodelist);
        nodelist
            .iter()
            .flat_map(|xs| self.render_node(xs))
            .collect::<Vec<Block>>()
    }

    /// Collect definitions for images and links, which can be referred to
    /// many times in a single markdown document.
    fn collect_definitions(&mut self, nodelist: &[Node]) {
        let mut links = HashMap::new();
        let mut images = HashMap::new();

        nodelist.iter().for_each(|xs| match xs {
            Node::Image(image) => {
                images.insert(image.alt.clone(), image.clone());
            }
            Node::Definition(definition) => {
                links.insert(definition.identifier.clone(), definition.url.clone());
            }
            _ => {}
        });

        self.links = links;
        self.images = images;
    }

    /// Render a node that becomes either a single Notion block or a vec of them.
    /// This is a little clunky.
    fn render_node(&mut self, node: &Node) -> Vec<Block> {
        match node {
            // Node::Root(_) => Vec::new(),
            Node::BlockQuote(quote) => self.render_quote(quote),
            Node::FootnoteDefinition(footnote) => self.render_footnote(footnote),
            Node::List(list) => self.begin_list(list),
            Node::Html(html) => self.render_html(html),
            Node::Image(image) => self.render_image(image),
            Node::ImageReference(imgref) => self.render_image_ref(imgref),
            Node::Code(code) => self.render_code(code),
            Node::Math(math) => self.render_math(math),
            Node::Heading(heading) => self.render_heading(heading),
            Node::Table(table) => self.begin_table(table),
            Node::TableRow(row) => self.table_row(row),
            Node::ThematicBreak(div) => self.render_divider(div),
            Node::ListItem(list_item) => self.render_list_item(list_item),
            Node::Paragraph(paragraph) => self.render_paragraph(paragraph),
            // All unhandled node types are deliberately skipped.
            _ => Vec::new(),
        }
    }

    /// Render a node type that becomes Notion rich text.
    fn render_text_node(&self, node: &Node) -> Option<Vec<RichText>> {
        match node {
            Node::Delete(deletion) => Some(self.render_deletion(deletion)),
            Node::Emphasis(emphasized) => Some(self.render_emphasized(emphasized)),
            Node::FootnoteReference(reference) => Some(vec![self.render_noteref(reference)]),
            Node::InlineCode(inline) => Some(vec![self.render_inline_code(inline)]),
            Node::InlineMath(math) => Some(vec![self.render_inline_math(math)]),
            Node::Link(link) => Some(vec![self.render_link(link)]),
            Node::LinkReference(linkref) => Some(vec![self.render_linkref(linkref)]),
            Node::Strong(strong) => Some(self.render_strong(strong)),
            Node::Text(text) => Some(self.render_text(text)),
            _ => None,
        }
    }

    // Repeat yourself to find patterns, I say, doggedly.

    /// Render plain text.
    fn render_text(&self, input: &mdast::Text) -> Vec<RichText> {
        let annotations = Annotations { ..Default::default() };
        State::split_text_at_api_limit(input.value.clone(), annotations)
    }

    /// Convenience for turning a text range into a rich text blob given a style annotation.
    fn make_into_rich_text(children: &[Node], style: Annotations) -> Vec<RichText> {
        let content: String = children
            .iter()
            .filter_map(|xs| match xs {
                Node::Text(ref t) => Some(t.value.clone()),
                _ => None,
            })
            .collect::<Vec<String>>()
            .join("");
        State::split_text_at_api_limit(content, style)
    }

    fn split_text_at_api_limit(mut content: String, style: Annotations) -> Vec<RichText> {
        let mut results: Vec<RichText> = Vec::new();
        while content.len() > 2000 {
            let mut split_point = 2000;
            while !content.is_char_boundary(split_point) {
                split_point -= 1;
            }
            let (first, last) = content.split_at(split_point);
            let text = Text {
                content: first.to_owned(),
                link: None,
            };
            results.push(RichText::Text {
                text,
                annotations: Some(style.clone()),
                plain_text: Some(first.to_owned()),
                href: None,
            });
            content = last.to_string();
        }

        let text = Text {
            content: content.clone(),
            link: None,
        };
        results.push(RichText::Text {
            text,
            annotations: Some(style),
            plain_text: Some(content),
            href: None,
        });

        results
    }

    fn render_strong(&self, strong: &mdast::Strong) -> Vec<RichText> {
        let annotations = Annotations {
            bold: true,
            ..Default::default()
        };
        State::make_into_rich_text(strong.children.as_slice(), annotations)
    }

    fn render_emphasized(&self, emphasized: &mdast::Emphasis) -> Vec<RichText> {
        let annotations = Annotations {
            italic: true,
            ..Default::default()
        };
        State::make_into_rich_text(emphasized.children.as_slice(), annotations)
    }

    fn render_deletion(&self, strike: &mdast::Delete) -> Vec<RichText> {
        let annotations = Annotations {
            strikethrough: true,
            ..Default::default()
        };
        State::make_into_rich_text(strike.children.as_slice(), annotations)
    }

    fn render_link(&self, mdlink: &mdast::Link) -> RichText {
        let content: String = mdlink
            .children
            .iter()
            .filter_map(|xs| match xs {
                Node::Text(ref t) => Some(t.value.clone()),
                _ => None,
            })
            .collect::<Vec<String>>()
            .join("");

        let url = if let Some(u) = self.links.get(&mdlink.url) {
            u.clone()
        } else {
            mdlink.url.clone()
        };

        let link = Link { url: url.clone() };
        let text = Text {
            content: content.clone(),
            link: Some(link),
        };
        RichText::Text {
            text,
            annotations: None,
            plain_text: Some(content),
            href: Some(url),
        }
    }

    fn render_linkref(&self, linkref: &mdast::LinkReference) -> RichText {
        let content: String = linkref
            .children
            .iter()
            .filter_map(|xs| match xs {
                Node::Text(ref t) => Some(t.value.clone()),
                _ => None,
            })
            .collect::<Vec<String>>()
            .join("");

        let url = if let Some(u) = self.links.get(&linkref.identifier) {
            u.clone()
        } else {
            linkref.identifier.clone()
        };

        let link = Link { url: url.clone() };
        let text = Text {
            content: content.clone(),
            link: Some(link),
        };
        RichText::Text {
            text,
            annotations: None,
            plain_text: Some(content),
            href: Some(url),
        }
    }

    fn render_inline_code(&self, inline: &mdast::InlineCode) -> RichText {
        let text = Text {
            content: inline.value.clone(),
            link: None,
        };
        let annotations = Annotations {
            code: true,
            ..Default::default()
        };
        RichText::Text {
            text,
            annotations: Some(annotations),
            plain_text: Some(inline.value.clone()),
            href: None,
        }
    }

    fn render_inline_math(&self, math: &mdast::InlineMath) -> RichText {
        let equation = Equation {
            expression: math.value.clone(),
        };
        let annotations = Annotations {
            code: true,
            ..Default::default()
        };

        RichText::Equation {
            equation,
            annotations,
            plain_text: math.value.clone(),
            href: None,
        }
    }

    fn render_quote(&self, quote: &mdast::BlockQuote) -> Vec<Block> {
        let rich_text: Vec<RichText> = quote
            .children
            .iter()
            .filter_map(|xs| self.render_text_node(xs))
            .flatten()
            .collect();
        let quote = QuoteValue {
            rich_text,
            color: TextColor::Default,
            children: None,
        };
        vec![Block {
            block_type: BlockType::Quote { quote },
            ..Default::default()
        }]
    }

    fn render_footnote(&self, footnote: &mdast::FootnoteDefinition) -> Vec<Block> {
        let rich_text = footnote
            .children
            .iter()
            .filter_map(|xs| self.render_text_node(xs))
            .flatten()
            .collect();
        let emoji = Emoji {
            emoji: "🗒️".to_string()
        };
        let icon = notion_client::objects::block::Icon::Emoji(emoji);
        let callout = CalloutValue {
            rich_text,
            icon,
            color: TextColor::Default,
        };
        vec![Block {
            block_type: BlockType::Callout { callout },
            ..Default::default()
        }]
    }

    /// Fragment links are a amajor PITA. You _can_ link to blocks, but you have to get their
    /// ids first, which means they have to be created first. So we're going to punt and make
    /// this look like a footnote, but not include the link part part of the WWW. How 1992 of us.
    fn render_noteref(&self, noteref: &mdast::FootnoteReference) -> RichText {
        let annotations = Annotations {
            color: notion_client::objects::rich_text::TextColor::Gray,
            ..Default::default()
        };
        let text = Text {
            content: noteref.identifier.clone(),
            link: None,
        };
        RichText::Text {
            text,
            annotations: Some(annotations),
            plain_text: Some(noteref.identifier.clone()),
            href: None,
        }
    }

    fn begin_table(&mut self, intable: &mdast::Table) -> Vec<Block> {
        let has_row_header = if let Some(_first) = intable.align.first() {
            // well, this is probably wrong, but I dunno if I am getting this info
            // with my current markdown parser settings. hrm.
            true
        } else {
            false
        };

        let children = self.render_nodes(intable.children.as_slice());

        // Now we look at children and find the row with the largest number of
        // cells. That's our table width.

        // TODO: Rows that are shorter than this need to be padded out.
        // Who knew markdown was so flexible and Notion so inflexible?
        // Answer: Anybody who looked at them both.

        let longest: u32 = children.iter().fold(1, |acc, xs| match &xs.block_type {
            BlockType::TableRow { table_row } => std::cmp::max(acc, table_row.cells.len() as u32),
            _ => acc,
        });

        let table = TableValue {
            table_width: longest,
            has_column_header: false,
            has_row_header,
            children: Some(children),
        };
        vec![Block {
            block_type: BlockType::Table { table },
            ..Default::default()
        }]
    }

    fn table_row(&self, row: &mdast::TableRow) -> Vec<Block> {
        let cells: Vec<Vec<RichText>> = row
            .children
            .iter()
            .filter_map(|xs| match xs {
                Node::TableCell(cell) => Some(self.table_cell(cell)),
                _ => None,
            })
            .collect();

        let table_row = TableRowsValue { cells };
        vec![Block {
            block_type: BlockType::TableRow { table_row },
            ..Default::default()
        }]
    }

    fn table_cell(&self, cell: &mdast::TableCell) -> Vec<RichText> {
        cell.children
            .iter()
            .filter_map(|xs| self.render_text_node(xs))
            .flatten()
            .collect()
    }

    fn render_paragraph(&self, para: &mdast::Paragraph) -> Vec<Block> {
        let rich_text: Vec<RichText> = para
            .children
            .iter()
            .filter_map(|xs| self.render_text_node(xs))
            .flatten()
            .collect();
        let paragraph = ParagraphValue {
            rich_text,
            color: Some(TextColor::Default),
            children: None,
        };
        vec![Block {
            block_type: BlockType::Paragraph { paragraph },
            ..Default::default()
        }]
    }

    fn render_code(&self, fenced: &mdast::Code) -> Vec<Block> {
        let language = if let Some(langstr) = fenced.lang.as_ref() {
            serde_json::from_str(langstr.as_str()).unwrap_or(Language::PlainText)
        } else {
            Language::PlainText
        };

        let text = Text {
            content: fenced.value.clone(),
            link: None,
        };
        let rich_text = RichText::Text {
            text,
            annotations: None,
            plain_text: Some(fenced.value.clone()),
            href: None,
        };
        let code = CodeValue {
            caption: Vec::new(),
            rich_text: vec![rich_text],
            language,
        };
        vec![Block {
            block_type: BlockType::Code { code },
            ..Default::default()
        }]
    }

    fn render_math(&self, math: &mdast::Math) -> Vec<Block> {
        let equation = EquationValue {
            expression: math.value.clone(),
        };
        vec![Block {
            block_type: BlockType::Equation { equation },
            ..Default::default()
        }]
    }

    // This is a hack. There really isn't an equivalent AFAICT.
    fn render_html(&self, html: &mdast::Html) -> Vec<Block> {
        let text = Text {
            content: html.value.clone(),
            link: None,
        };
        let rich_text = RichText::Text {
            text,
            annotations: None,
            plain_text: Some(html.value.clone()),
            href: None,
        };
        let code = CodeValue {
            caption: Vec::new(),
            rich_text: vec![rich_text],
            language: Language::PlainText,
        };
        vec![Block {
            block_type: BlockType::Code { code },
            ..Default::default()
        }]
    }

    /// Img block pointing to a previously declared image.
    fn render_image_ref(&self, imgref: &mdast::ImageReference) -> Vec<Block> {
        if let Some(image) = self.images.get(&imgref.identifier) {
            self.render_image(image)
        } else {
            vec![Block {
                block_type: BlockType::None,
                ..Default::default()
            }]
        }
    }

    fn render_image(&self, image: &mdast::Image) -> Vec<Block> {
        // TODO: For now. What we should do is figure out if this is a local image and upload
        // if so and make a local file url.
        let external = ExternalFile { url: image.url.clone() };
        let file_type = File::External { external };
        let image = ImageValue { file_type };
        vec![Block {
            block_type: BlockType::Image { image },
            ..Default::default()
        }]
    }

    fn begin_list(&mut self, list: &mdast::List) -> Vec<Block> {
        let mut state = self.clone();
        state.list = if list.ordered {
            ListVariation::Ordered
        } else {
            ListVariation::Bulleted
        };
        if let Some(start) = list.start {
            state.ordered_start = start;
        }
        state.render_nodes(list.children.as_slice())
    }

    fn render_list_item(&mut self, item: &mdast::ListItem) -> Vec<Block> {
        match self.list {
            ListVariation::None => self.rendered_bullet_li(item),
            ListVariation::Bulleted => self.rendered_bullet_li(item),
            ListVariation::Ordered => self.render_numbered_li(item),
        }
    }

    // TODO these two list item functions have a lot in common, you know?

    fn render_numbered_li(&mut self, item: &mdast::ListItem) -> Vec<Block> {
        let mut children: VecDeque<Node> = VecDeque::from(item.children.clone());
        let Some(first) = children.pop_front() else {
            // we can short-circuit. Empty list.
            let numbered_list_item = NumberedListItemValue {
                rich_text: Vec::new(),
                color: TextColor::Default,
                children: None,
            };
            return vec![Block {
                block_type: BlockType::NumberedListItem { numbered_list_item },
                ..Default::default()
            }];
        };

        let rich_text: Vec<RichText> = match first {
            Node::Paragraph(paragraph) => paragraph
                .children
                .iter()
                .filter_map(|xs| self.render_text_node(xs))
                .flatten()
                .collect(),
            _ => Vec::new(),
        };

        let block_kids: Vec<Block> = self.render_nodes(&Vec::from(children));
        let numbered_list_item = NumberedListItemValue {
            rich_text,
            color: TextColor::Default,
            children: Some(block_kids),
        };
        vec![Block {
            block_type: BlockType::NumberedListItem { numbered_list_item },
            ..Default::default()
        }]
    }

    fn rendered_bullet_li(&mut self, item: &mdast::ListItem) -> Vec<Block> {
        let mut children: VecDeque<Node> = VecDeque::from(item.children.clone());
        let Some(first) = children.pop_front() else {
            // we can short-circuit. Empty list.
            let bulleted_list_item = BulletedListItemValue {
                rich_text: Vec::new(),
                color: TextColor::Default,
                children: None,
            };
            return vec![Block {
                block_type: BlockType::BulletedListItem { bulleted_list_item },
                ..Default::default()
            }];
        };

        let rich_text: Vec<RichText> = match first {
            Node::Paragraph(paragraph) => paragraph
                .children
                .iter()
                .filter_map(|xs| self.render_text_node(xs))
                .flatten()
                .collect(),
            _ => Vec::new(),
        };

        let block_kids: Vec<Block> = self.render_nodes(&Vec::from(children));
        let bulleted_list_item = BulletedListItemValue {
            rich_text,
            color: TextColor::Default,
            children: Some(block_kids),
        };
        vec![Block {
            block_type: BlockType::BulletedListItem { bulleted_list_item },
            ..Default::default()
        }]
    }

    fn render_divider(&self, _thematic: &mdast::ThematicBreak) -> Vec<Block> {
        let divider = DividerValue {};
        vec![Block {
            block_type: BlockType::Divider { divider },
            ..Default::default()
        }]
    }

    fn render_heading(&self, heading: &mdast::Heading) -> Vec<Block> {
        let rich_text: Vec<RichText> = heading
            .children
            .iter()
            .filter_map(|xs| self.render_text_node(xs))
            .flatten()
            .collect();

        let value = HeadingsValue {
            rich_text,
            ..Default::default()
        };
        let block_type = if heading.depth == 1 {
            BlockType::Heading1 { heading_1: value }
        } else if heading.depth == 2 {
            BlockType::Heading2 { heading_2: value }
        } else {
            BlockType::Heading3 { heading_3: value }
        };

        vec![Block {
            block_type,
            ..Default::default()
        }]
    }
}

fn split_block_from_children(block: Block) -> (Block, Option<VecDeque<Block>>) {
    // There are many block types here that we skip because we are never
    // generating them while converting from markdown. We also skip block
    // types that do not have a `children` field.
    let maybe_kids = match block.block_type {
        BlockType::BulletedListItem { ref bulleted_list_item } => &bulleted_list_item.children,
        BlockType::NumberedListItem { ref numbered_list_item } => &numbered_list_item.children,
        BlockType::Paragraph { ref paragraph } => &paragraph.children,
        BlockType::Quote { ref quote } => &quote.children,
        _ => &None,
    };
    let Some(children) = maybe_kids else {
        return (block, None);
    };
    if children.is_empty() {
        return (block, None);
    }
    let mut replacement = block.clone();
    replacement.has_children = Some(false);
    match block.block_type {
        BlockType::BulletedListItem { ref bulleted_list_item } => {
            let mut bulleted_list_item = bulleted_list_item.clone();
            bulleted_list_item.children = None;
            replacement.block_type = BlockType::BulletedListItem { bulleted_list_item };
        }
        BlockType::NumberedListItem { ref numbered_list_item } => {
            let mut numbered_list_item = numbered_list_item.clone();
            numbered_list_item.children = None;
            replacement.block_type = BlockType::NumberedListItem { numbered_list_item };
        }
        BlockType::Paragraph { ref paragraph } => {
            let mut paragraph = paragraph.clone();
            paragraph.children = None;
            replacement.block_type = BlockType::Paragraph { paragraph };
        }
        BlockType::Quote { ref quote } => {
            let mut quote = quote.clone();
            quote.children = None;
            replacement.block_type = BlockType::Quote { quote };
        }
        _ => {}
    }
    (replacement, Some(VecDeque::from(children.clone())))
}

#[cfg(test)]
mod libtest {
    use super::*;

    #[test]
    #[ignore]
    fn delving() {
        let input = include_str!("../fixtures/nested_lists.md");
        let blocks = convert(input);
        blocks.iter().for_each(|xs| {
            crate::tests::debug_print(xs);
            if PageMaker::block_has_deep_children(0, xs) {
                eprintln!("    ^^^^ too deep!");
            }
        });
        // assert_eq!(true, false);
    }

    /// This creates a page. Be sure you want this.
    #[tokio::test]
    #[ignore]
    async fn creating_by_chunks() {
        let _ignored = dotenvy::dotenv().unwrap();
        let notion_key =
            std::env::var("NOTION_API_KEY").expect("The test creating_by_chunks needs the env var NOTION_API_KEY.");
        let notion = Client::new(notion_key, None).expect("should be able to make a client");
        let parent =
            std::env::var("PARENT_ID").expect("The test creating_by_chunks needs a parent page id in PARENT_ID");

        let mut properties: BTreeMap<String, PageProperty> = BTreeMap::new();
        let text = Text {
            content: "Nested list test".to_string(),
            link: None,
        };
        let title = vec![RichText::Text {
            text,
            annotations: None,
            plain_text: Some("Nested list test".to_string()),
            href: None,
        }];
        properties.insert("title".to_string(), PageProperty::Title { id: None, title });

        let input = include_str!("../fixtures/nested_lists.md");
        let page = create_page(&notion, input, parent.as_str(), properties)
            .await
            .expect("create_page() should succeed in testing");
        assert!(!page.id.is_empty());
        assert!(matches!(page.parent, Parent::PageId { .. }));
        match page.parent {
            Parent::None => {}
            Parent::PageId { page_id } => {
                assert_eq!(page_id, parent);
            }
            Parent::BlockId { .. } => {}
            Parent::Workspace { .. } => {}
            Parent::DatabaseId { .. } => {}
        }
    }
}
