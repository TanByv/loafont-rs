use super::*;

#[test]
fn parses_and_rebuilds() {
    let xml = br#"<?xml version="1.0"?>
<FontMap>
    <Item Key="$A" File="a.ttf" SizeCorrectionRatio="120" LeadingCorrection="-200" />
    <Item Key="$B" File="b.ttf" Extra="x&amp;y" />
</FontMap>"#;
    let items = parse(xml).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].size, "120");
    assert_eq!(items[0].leading, "-200");
    assert_eq!(items[1].extra["Extra"], "x&y");
    validate(&items).unwrap();

    let rebuilt = build(&items);
    let reparsed = parse(rebuilt.as_bytes()).unwrap();
    assert_eq!(reparsed.len(), 2);
    assert_eq!(reparsed[1].extra["Extra"], "x&y");
    assert_eq!(reparsed[0].size, "120");
}

#[test]
fn rejects_duplicate_and_invalid_rows() {
    let duplicate =
        br#"<FontMap><Item Key="$A" File="a.ttf" /><Item Key="$A" File="b.ttf" /></FontMap>"#;
    assert!(validate(&parse(duplicate).unwrap()).is_err());

    let invalid =
        br#"<FontMap><Item Key="$A" File="a.ttf" SizeCorrectionRatio="large" /></FontMap>"#;
    assert!(validate(&parse(invalid).unwrap()).is_err());

    let empty = br#"<FontMap></FontMap>"#;
    assert!(validate(&parse(empty).unwrap()).is_err());
}
