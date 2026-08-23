// SPDX-License-Identifier: Apache-2.0
//! Assertions that the repository's claims about itself are true.
//!
//! Documentation rots quietly. A crate table listing something that does not
//! exist, a link to a moved ADR, a rule citing a file nobody kept — none of those
//! break a build, and all of them cost the reader the benefit of the doubt on
//! everything else in the document.
//!
//! This crate is why `AGENTS.md` can say "verify before you claim" and mean it.
//! It exists because the failure it prevents already happened once, in a sibling
//! repository, and is recorded in `LEARNINGS.md`: a design was documented as
//! canonical, citing functions in a file that was not in the working tree and not
//! in git.
//!
//! It caught its own version of that on the first run. Three crate directories
//! existed with no manifest — including this one — and the two that were never
//! going to be written were deleted rather than left to look like work in
//! progress.
//!
//! # What is checked
//!
//! Only things that are mechanically decidable. A test that tries to judge
//! whether prose is *accurate* would either be wrong or be a second copy of the
//! prose; these check that every artefact the prose points at is really there.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The repository root, found by walking up from this crate.
///
/// # Panics
///
/// Panics if the root cannot be located, which means the crate has been moved
/// and every path below it is wrong anyway.
#[must_use]
pub fn root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !dir.join("Cargo.toml").exists() || !dir.join("crates").is_dir() {
        assert!(dir.pop(), "no repository root above the manifest directory");
    }
    dir
}

/// Directories under `crates/`.
///
/// # Panics
///
/// Panics if `crates/` cannot be read.
#[must_use]
pub fn crate_directories() -> BTreeSet<String> {
    std::fs::read_dir(root().join("crates"))
        .expect("crates/ must be readable")
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

/// Crates listed in the workspace `members` array.
///
/// Parsed by hand rather than with a TOML crate: this is one array of quoted
/// strings on one line, and a dependency added to check a dependency list is a
/// dependency that has to be justified.
///
/// # Panics
///
/// Panics if the root manifest has no `members` array.
#[must_use]
pub fn workspace_members() -> BTreeSet<String> {
    let manifest = std::fs::read_to_string(root().join("Cargo.toml"))
        .expect("the root manifest must be readable");
    let start = manifest
        .find("members = [")
        .expect("the root manifest must declare workspace members");
    let rest = &manifest[start..];
    let end = rest.find(']').expect("the members array must be closed");

    rest[..end]
        .split('"')
        .filter(|s| s.starts_with("crates/"))
        .map(|s| s.trim_start_matches("crates/").to_owned())
        .collect()
}

/// Relative markdown links found in a document, excluding anchors and URLs.
#[must_use]
pub fn relative_links(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = markdown.chars().collect();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == ']' && bytes.get(i + 1) == Some(&'(') {
            let mut j = i + 2;
            let mut target = String::new();
            while j < bytes.len() && bytes[j] != ')' {
                target.push(bytes[j]);
                j += 1;
            }
            i = j;
            let target = target.split_whitespace().next().unwrap_or("").to_owned();
            if !target.is_empty()
                && !target.starts_with('#')
                && !target.contains("://")
                && !target.starts_with("mailto:")
            {
                // Strip a trailing anchor: the file is what must exist.
                out.push(target.split('#').next().unwrap_or(&target).to_owned());
            }
        }
        i += 1;
    }
    out
}

/// Every markdown file tracked in the repository's documented areas.
///
/// # Panics
///
/// Panics if a directory that should exist cannot be read.
#[must_use]
pub fn documents() -> Vec<PathBuf> {
    let root = root();
    let mut out = vec![
        root.join("README.md"),
        root.join("AGENTS.md"),
        root.join("CLAUDE.md"),
        root.join("LEARNINGS.md"),
    ];
    for dir in ["docs/adr", "docs/research", "deploy"] {
        let path = root.join(dir);
        if !path.is_dir() {
            continue;
        }
        let mut found: Vec<PathBuf> = std::fs::read_dir(&path)
            .expect("a documented directory must be readable")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        found.sort();
        out.append(&mut found);
    }
    out.retain(|p| p.exists());
    out
}

/// Resolves a link found in `document` against the repository.
#[must_use]
pub fn resolve(document: &Path, link: &str) -> PathBuf {
    document
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_crate_directory_is_a_workspace_member() {
        // The check that would have caught three empty scaffolds sitting in
        // crates/ looking like work in progress, one of which was this crate.
        let members = workspace_members();
        let missing: Vec<String> = crate_directories()
            .into_iter()
            .filter(|name| !members.contains(name))
            .collect();
        assert!(
            missing.is_empty(),
            "these directories are in crates/ but not workspace members: {missing:?}\n\
             Either add them to the members array, or delete them — a directory named \
             after a crate that does not exist reads as work in progress."
        );
    }

    #[test]
    fn every_workspace_member_has_a_manifest() {
        for name in workspace_members() {
            let manifest = root().join("crates").join(&name).join("Cargo.toml");
            assert!(
                manifest.exists(),
                "workspace member `{name}` has no Cargo.toml at {}",
                manifest.display()
            );
        }
    }

    #[test]
    fn every_crate_has_source() {
        // A manifest with no source is a crate that compiles to nothing while
        // appearing in every listing.
        for name in workspace_members() {
            let src = root().join("crates").join(&name).join("src");
            let has_source = std::fs::read_dir(&src).is_ok_and(|entries| {
                entries
                    .filter_map(Result::ok)
                    .any(|e| e.path().extension().is_some_and(|x| x == "rs"))
            });
            assert!(has_source, "crate `{name}` has no Rust source in src/");
        }
    }

    #[test]
    fn every_relative_link_in_the_documentation_resolves() {
        // The failure this crate exists for: a document citing a file that is
        // not in the tree and not in git.
        let mut broken = Vec::new();
        for document in documents() {
            let Ok(text) = std::fs::read_to_string(&document) else {
                continue;
            };
            for link in relative_links(&text) {
                let target = resolve(&document, &link);
                if !target.exists() {
                    broken.push(format!("{} -> {link}", document.display()));
                }
            }
        }
        assert!(
            broken.is_empty(),
            "broken links:\n  {}",
            broken.join("\n  ")
        );
    }

    #[test]
    fn every_adr_referenced_by_number_exists() {
        // ADRs are cited as "ADR 0003" in prose as often as they are linked, and
        // a number with no file behind it is worse than no citation.
        let adr_dir = root().join("docs/adr");
        let numbers: BTreeSet<String> = std::fs::read_dir(&adr_dir)
            .expect("docs/adr must be readable")
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|name| name.split('-').next().map(ToOwned::to_owned))
            .filter(|n| n.len() == 4 && n.chars().all(|c| c.is_ascii_digit()))
            .collect();

        let mut missing = Vec::new();
        for document in documents() {
            let Ok(text) = std::fs::read_to_string(&document) else {
                continue;
            };
            for (index, _) in text.match_indices("ADR ") {
                let cited: String = text[index + 4..]
                    .chars()
                    .take(4)
                    .filter(char::is_ascii_digit)
                    .collect();
                if cited.len() == 4 && !numbers.contains(&cited) {
                    missing.push(format!("{} cites ADR {cited}", document.display()));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "citations with no ADR behind them:\n  {}",
            missing.join("\n  ")
        );
    }

    #[test]
    fn the_readme_crate_table_matches_the_workspace() {
        // A table listing a crate that does not exist, or omitting one that
        // does, is the most-read wrong thing in the repository.
        let readme =
            std::fs::read_to_string(root().join("README.md")).expect("README must be readable");
        for name in workspace_members() {
            if name == "repo-conformance" {
                // Deliberately absent from the table: it ships no capability,
                // and listing the invigilator among the players reads oddly.
                continue;
            }
            assert!(
                readme.contains(&format!("`{name}`")),
                "crate `{name}` exists but the README's table does not mention it"
            );
        }
    }

    #[test]
    fn the_deploy_units_referenced_by_the_deploy_guide_exist() {
        // Instructions that install a file which is not there fail on a
        // production box, at the worst moment, in front of whoever was trusted
        // with the access.
        let guide = root().join("deploy/README.md");
        let Ok(text) = std::fs::read_to_string(&guide) else {
            return;
        };
        let mut missing = Vec::new();
        for (index, _) in text.match_indices("deploy/") {
            let path: String = text[index..]
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '`' && *c != ')')
                .collect();
            if !root().join(&path).exists() {
                missing.push(path);
            }
        }
        assert!(
            missing.is_empty(),
            "the deploy guide names files that do not exist: {missing:?}"
        );
    }

    #[test]
    fn links_are_extracted_the_way_a_reader_would_read_them() {
        // The extractor itself, checked — otherwise a bug here silently turns
        // every assertion above into a test that passes by finding nothing.
        let markdown = "See [one](docs/a.md) and [two](docs/b.md#section), \
                        [ext](https://example.com), [anchor](#here).";
        assert_eq!(
            relative_links(markdown),
            vec!["docs/a.md".to_owned(), "docs/b.md".to_owned()]
        );
    }

    #[test]
    fn the_extractor_finds_something_in_the_real_documents() {
        // Guards against the failure mode where every link test passes because
        // no links were found at all.
        let total: usize = documents()
            .iter()
            .filter_map(|d| std::fs::read_to_string(d).ok())
            .map(|text| relative_links(&text).len())
            .sum();
        assert!(
            total > 10,
            "only {total} relative links found across the docs"
        );
    }
}
