#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn rich_text() {
        let input = "This _markdown_ file has *only* some `text` styles in it, ~not much~ nothing more.";
        let result = convert(input);
        assert_eq!(result.len(), 1);
        let block = result.first().expect("we expect one block");
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
