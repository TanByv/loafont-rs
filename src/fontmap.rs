use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FontItem {
    pub key: String,
    pub file: String,
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub leading: String,
    #[serde(default)]
    pub extra: BTreeMap<String, String>,
}

pub fn parse(data: &[u8]) -> Result<Vec<FontItem>> {
    let text = std::str::from_utf8(data).context("FontMap.xml is not valid UTF-8.")?;
    let document = roxmltree::Document::parse(text).context("FontMap.xml is not valid XML.")?;

    if !document.root_element().has_tag_name("FontMap") {
        bail!(
            "FontMap.xml has a <{}> root, expected <FontMap>.",
            document.root_element().tag_name().name()
        );
    }

    let mut items = Vec::new();
    for node in document
        .descendants()
        .filter(|node| node.has_tag_name("Item"))
    {
        let mut item = FontItem::default();

        for attribute in node.attributes() {
            match attribute.name() {
                "Key" => item.key = attribute.value().to_string(),
                "File" => item.file = attribute.value().to_string(),
                "SizeCorrectionRatio" => item.size = attribute.value().to_string(),
                "LeadingCorrection" => item.leading = attribute.value().to_string(),
                other => {
                    item.extra
                        .insert(other.to_string(), attribute.value().to_string());
                }
            }
        }

        items.push(item);
    }

    Ok(items)
}

pub fn validate(items: &[FontItem]) -> Result<()> {
    if items.is_empty() {
        bail!("FontMap has no rows.");
    }

    let mut keys = std::collections::HashSet::new();
    for item in items {
        if item.key.trim().is_empty() {
            bail!("A FontMap row has an empty Key.");
        }
        if item.file.trim().is_empty() {
            bail!("FontMap row {} has an empty File.", item.key);
        }
        if !keys.insert(item.key.clone()) {
            bail!("FontMap contains a duplicate Key: {}", item.key);
        }
        for (label, value) in [
            ("Size correction", item.size.trim()),
            ("Leading correction", item.leading.trim()),
        ] {
            if !value.is_empty() && value.parse::<f64>().is_err() {
                bail!(
                    "FontMap row {}: {label} '{value}' is not a number.",
                    item.key
                );
            }
        }
    }

    Ok(())
}

pub fn build(items: &[FontItem]) -> String {
    let mut lines = vec![
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>".to_string(),
        "<FontMap>".to_string(),
    ];

    for item in items {
        let mut attributes = vec![
            ("Key".to_string(), item.key.trim().to_string()),
            ("File".to_string(), item.file.trim().to_string()),
        ];

        if !item.size.trim().is_empty() {
            attributes.push((
                "SizeCorrectionRatio".to_string(),
                item.size.trim().to_string(),
            ));
        }
        if !item.leading.trim().is_empty() {
            attributes.push((
                "LeadingCorrection".to_string(),
                item.leading.trim().to_string(),
            ));
        }
        for (key, value) in &item.extra {
            attributes.push((key.clone(), value.clone()));
        }

        let attributes = attributes
            .into_iter()
            .map(|(key, value)| format!("{key}=\"{}\"", escape(&value)))
            .collect::<Vec<_>>()
            .join(" ");

        lines.push(format!("    <Item {attributes} />"));
    }

    lines.push("</FontMap>".to_string());
    lines.join("\n") + "\n"
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
#[path = "../tests/unit/fontmap.rs"]
mod tests;
