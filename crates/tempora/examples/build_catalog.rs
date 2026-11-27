//! Builds the WinGet category catalog that Tempora embeds.
//!
//! ```text
//! cargo run -p tempora --example build_catalog -- <packages.jsonl> <labels.tsv> <out .bin>
//! ```
//!
//! `packages.jsonl` comes from `scripts/catalog/extract.py`, `labels.tsv` from
//! `scripts/catalog/merge.py`. Keys come from `Package::keys`, the same code the recorder's lookups
//! use. A key that leads to packages with different categories is dropped, and packages without
//! a category are left out.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fs;

use serde_json::Value;
use tempora::model::Category;
use tempora::recognition::catalog::{Package, encode};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [packages_path, labels_path, out_path] = args.as_slice() else {
        return Err("usage: build_catalog <packages.jsonl> <labels.tsv> <out .bin>".into());
    };

    let labels_text = fs::read_to_string(labels_path)?;
    let labels: HashMap<&str, Category> = labels_text
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split('\t');
            let package = columns.next()?;
            let label = columns.next()?;
            let category = Category::ALL
                .into_iter()
                .find(|category| category.label() == label)?;
            Some((package, category))
        })
        .collect();

    let mut by_key = BTreeMap::<String, BTreeSet<Category>>::new();
    let (mut packages, mut labelled) = (0, 0);
    for line in fs::read_to_string(packages_path)?.lines() {
        let record: Value = serde_json::from_str(line)?;
        packages += 1;
        let Some(category) = record["id"].as_str().and_then(|id| labels.get(id)) else {
            continue;
        };
        labelled += 1;
        for key in package(&record).keys() {
            by_key.entry(key).or_default().insert(*category);
        }
    }

    let settled: Vec<(&str, Category)> = by_key
        .iter()
        .filter_map(
            |(key, categories)| match categories.iter().collect::<Vec<_>>()[..] {
                [category] => Some((key.as_str(), *category)),
                _ => None,
            },
        )
        .collect();
    let data = encode(settled.iter().copied());
    fs::write(out_path, &data)?;

    let mut kinds = BTreeMap::<char, usize>::new();
    for (key, _) in &settled {
        *kinds.entry(key.chars().next().unwrap_or('?')).or_default() += 1;
    }
    let kinds: Vec<String> = kinds
        .iter()
        .map(|(kind, count)| format!("{kind}: {count}"))
        .collect();
    println!("packages: {packages}, with a category: {labelled}");
    println!(
        "keys: {} ({}), dropped as conflicting: {}",
        settled.len(),
        kinds.join(", "),
        by_key.len() - settled.len()
    );
    println!("wrote {out_path}: {} KiB", data.len().div_ceil(1024));
    Ok(())
}

/// The manifest fields `extract.py` wrote for one package.
fn package(record: &Value) -> Package<'_> {
    let list = |name: &str| -> Vec<&str> {
        record["keys"][name]
            .as_array()
            .map(|values| values.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    };
    let with = |first: &str, rest: &str| -> Vec<&str> {
        let mut values: Vec<&str> = record[first].as_str().into_iter().collect();
        values.extend(list(rest));
        values.sort_unstable();
        values.dedup();
        values.retain(|value| !value.is_empty());
        values
    };
    let mut commands = list("PortableCommandAlias");
    commands.extend(list("Commands"));
    Package {
        names: with("name", "DisplayName"),
        publishers: with("publisher", "Publisher"),
        product_codes: list("ProductCode"),
        families: list("PackageFamilyName"),
        file_paths: list("RelativeFilePath"),
        commands,
    }
}
