#[cfg(test)]
mod tests {
    use notion_client::objects::block::*;
    use nuc2not::convert;

    #[derive(Debug, Clone)]
    struct MockClient {
        // todo
    }

    #[test]
    fn rich_text() {
        let input = "This _markdown_ file has *only* some `text` styles in it, ~not much~ nothing more.";
        let result = convert(input);
        assert_eq!(result.len(), 1);
        let block = result.first().expect("we really expected a paragraph here");
        let paragraph = match &block.block_type {
            BlockType::Paragraph { paragraph } => paragraph,
            _ => {
                panic!("expected a paragraph");
            }
        };
        assert!(paragraph.children.is_none());
        assert_eq!(paragraph.rich_text.len(), 9);
    }

    #[test]
    fn bulleted_list() {
        let input = include_str!("../fixtures/bulleted_list.md");
        let result = convert(input);
        assert_eq!(result.len(), 4);
        result.iter().for_each(|xs| {
            let _item = match &xs.block_type {
                BlockType::BulletedListItem { bulleted_list_item } => bulleted_list_item,
                _ => {
                    panic!("expected a bulleted list item; got {xs:?}");
                }
            };
        });
    }

    #[test]
    fn ordered_list() {
        let input = include_str!("../fixtures/ordered_list.md");
        let blocks = convert(input);
        assert_eq!(blocks.len(), 8);

        blocks.iter().for_each(|xs| {
            let item = match &xs.block_type {
                BlockType::NumberedListItem { numbered_list_item } => numbered_list_item,
                _ => {
                    panic!("expected a numbered list item; got {xs:?}");
                }
            };
            eprintln!("{item:?}");
            assert!(item.children.is_some());
        });
    }

    #[test]
    fn nested_lists() {
        let input = include_str!("../fixtures/nested_lists.md");
        let blocks = convert(input);
        assert_eq!(blocks.len(), 3);
        let first_item = match &blocks[0].block_type {
            BlockType::BulletedListItem { bulleted_list_item } => bulleted_list_item,
            _ => {
                panic!("expected a bulleted list item");
            }
        };
        let sublist_1 = first_item
            .children
            .as_ref()
            .expect("first list item should have a sublist");
        // First child is a paragraph block, aka the list item content.
        // Second child is a nested list.
        let sublist_1_first = match &sublist_1[1].block_type {
            BlockType::BulletedListItem { bulleted_list_item } => bulleted_list_item,
            _ => {
                panic!("expected a bulleted list item");
            }
        };
        assert!(sublist_1_first.children.is_some());
        let sublist_2 = sublist_1_first
            .children
            .as_ref()
            .expect("this list item should have children");
        assert_eq!(sublist_2.len(), 3);
        // Same for this next list: paragraph, then a nested list.
        let sublist_2_second = match &sublist_2[1].block_type {
            BlockType::BulletedListItem { bulleted_list_item } => bulleted_list_item,
            _ => {
                panic!("expected a bulleted list item");
            }
        };
        assert!(sublist_2_second.children.is_some());
        let sublist_3 = sublist_2_second
            .children
            .as_ref()
            .expect("this list item should have children");
        assert_eq!(sublist_3.len(), 3);
    }

    #[test]
    fn headers() {
        let input = include_str!("../fixtures/headers_and_grafs.md");
        let blocks = convert(input);
        assert_eq!(blocks.len(), 10);
        blocks.iter().for_each(|xs| match &xs.block_type {
            BlockType::Heading1 { heading_1 } => {
                assert!(!heading_1.rich_text.is_empty());
                assert_eq!(heading_1.rich_text.len(), 1);
            }
            BlockType::Heading2 { heading_2 } => {
                assert_eq!(heading_2.rich_text.len(), 2);
            }
            BlockType::Heading3 { heading_3 } => {
                assert!(!heading_3.rich_text.is_empty());
            }
            BlockType::Paragraph { paragraph } => {
                assert!(!paragraph.rich_text.is_empty());
            }
            _ => {
                panic!("unexpected block type! {:?}", xs.block_type);
            }
        });
    }

    #[test]
    fn references() {
        let input = include_str!("../fixtures/references.md");
        let result = convert(input);
        result.iter().for_each(|xs| {
            eprintln!("{xs:?}");
        });
    }

    #[test]
    fn most_syntax() {
        let input = include_str!("../fixtures/more_complex.md");
        let result = convert(input);
        assert_eq!(result.len(), 102);
    }

    #[test]
    fn complex_with_tables() {
        let input = include_str!("../fixtures/table.md");
        let result = convert(input);
        assert_eq!(result.len(), 16);
        let table = result[10].clone();
        assert!(matches!(table.block_type, BlockType::Table { .. }));
    }

    #[test]
    fn gfm_parsing() {
        let input = include_str!("../fixtures/gfm-test.md");
        let result = convert(input);
        assert_eq!(result.len(), 36);
        let block10 = result[10].clone();
        eprintln!("{block10:?}");
        assert!(matches!(block10.block_type, BlockType::Heading3 { .. }));
        let _heading3 = match block10.block_type {
            BlockType::Heading3 { heading_3 } => heading_3,
            _ => {
                panic!("expected heading 3 block type");
            }
        };
    }
}
