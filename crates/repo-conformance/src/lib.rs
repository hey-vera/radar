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

    // Bounded by the input length rather than by trusting the cursor to
    // advance -- see `test_paths` below, which had the same shape and was
    // reported by CI as two five-minute timeouts before it was changed.
    for _ in 0..=bytes.len() {
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == ']' && bytes.get(i + 1) == Some(&'(') {
            let mut j = i + 2;
            let mut target = String::new();
            for _ in 0..=bytes.len() {
                if j >= bytes.len() || bytes[j] == ')' {
                    break;
                }
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

/// Every markdown document in the repository.
///
/// **Discovered, not listed.** This was a hand-written enumeration of four root
/// files plus `docs/adr`, `docs/research` and `deploy`, and `docs/` itself was
/// not among them — so `docs/STATE.md` was created, committed, and checked by
/// nothing. Fifteen of its links were broken on the day it landed, because it
/// had been cut out of root-level `AGENTS.md` and its `docs/...` paths were
/// never re-based; every conformance rule passed, because none of them looked.
///
/// A list of what to check is a second thing to keep in sync with the tree, and
/// it fails silently in the direction that reports success. Deriving it from
/// [`known_files`] cannot go stale: a document added anywhere is checked from
/// the commit that adds it.
/// Every markdown file tracked in the repository's documented areas.
///
/// # Panics
///
/// Panics if a directory that should exist cannot be read.
#[must_use]
pub fn documents() -> Vec<PathBuf> {
    let root = root();
    let mut out: Vec<PathBuf> = known_files()
        .iter()
        // Extension rather than a suffix match, and case-insensitively. On
        // Windows `AGENTS.MD` and `AGENTS.md` are the same file, and this
        // repository has already lost a document to exactly that collision --
        // so a `.MD` is a document worth checking, not one worth skipping.
        .filter(|f| {
            Path::new(f.as_str())
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        })
        .map(|f| root.join(f))
        .collect();
    out.retain(|p| p.exists());
    out.sort();
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

/// Every file this repository is made of, relative to its root, `/`-separated.
///
/// Git first, because the failure this supports is a file that exists on
/// somebody's disk and in no commit — and only git can tell those apart.
///
/// Falling back to walking the tree when git cannot answer is deliberate and is
/// weaker on purpose. `cargo mutants` builds in a copy with no `.git`, and a
/// check that skipped itself there would be vacuous exactly where it is least
/// observed. Existence is a smaller claim than tracked-ness, and it is a claim.
#[must_use]
pub fn known_files() -> BTreeSet<String> {
    let from_git = std::process::Command::new("git")
        .arg("-C")
        .arg(root())
        .args(["ls-files"])
        .output();
    if let Ok(out) = from_git
        && out.status.success()
    {
        {
            let files = parse_ls_files(&String::from_utf8_lossy(&out.stdout));
            // A mutant deleting this `!` survives, and it is worth writing down
            // rather than chasing: inverted, a successful `git ls-files` falls
            // through to walking the tree, which produces an equally usable
            // list. The two branches disagree about *provenance* — tracked
            // versus merely present — and no test can see that difference from
            // inside a checkout where everything present is also tracked.
            if !files.is_empty() {
                return files;
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(&root(), &root(), &mut out);
    out
}

/// Parses `git ls-files` output into repository-relative paths.
///
/// Split out from the process call because the process call cannot be tested and
/// this can. Blank lines are dropped and separators normalised to `/`, both of
/// which matter: an empty entry would match every suffix query and quietly make
/// the check that uses this pass for any path at all.
#[must_use]
pub fn parse_ls_files(output: &str) -> BTreeSet<String> {
    output
        .lines()
        .map(|l| l.trim().replace('\\', "/"))
        .filter(|l| !l.is_empty())
        .collect()
}

/// Collects every file under `dir` as a path relative to `base`.
fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut BTreeSet<String>) {
    // `target` is build output and `.git` is not source. Descending into either
    // would take minutes and find nothing a document would ever name.
    const SKIP: &[&str] = &["target", ".git", "node_modules"];
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if SKIP.iter().any(|s| *s == name) {
            continue;
        }
        if path.is_dir() {
            walk(&path, base, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.insert(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Paths named inside backticks in a document.
///
/// Deliberately conservative. A code span holds all sorts of things — type
/// names, flags, function calls — and flagging those would make the check
/// noisy enough to be turned off, which is worse than not having it. So a span
/// counts as a path only if it looks like one and nothing else: it contains a
/// `/` or a known file extension, has no whitespace, parentheses or leading
/// dashes, and does not end in `()`.
#[must_use]
pub fn code_span_paths(text: &str) -> BTreeSet<String> {
    const EXTENSIONS: &[&str] = &[".rs", ".toml", ".md", ".yml", ".yaml", ".service", ".timer"];
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else { break };
        let span = &after[..end];
        rest = &after[end + 1..];

        let looks_like_a_path = (span.contains('/')
            || EXTENSIONS.iter().any(|e| span.ends_with(e)))
            && !span.contains(char::is_whitespace)
            && !span.contains('(')
            && !span.starts_with('-')
            && !span.contains('*')
            // `other-repo:path/to/file` names a file somewhere else, and a
            // leading `/` names one on a machine. Neither is a claim about this
            // repository, and both are how such a claim should be written --
            // which is the point, because LEARNINGS entry 1 is about a citation
            // that gave a reader no way to tell.
            && !span.contains(':')
            && !span.starts_with('/');
        // A bare `a/b` with no extension is as likely to be prose as a path.
        let has_extension = EXTENSIONS.iter().any(|e| span.ends_with(e));
        if looks_like_a_path && has_extension {
            out.insert(span.to_owned());
        }
    }
    out
}

/// Lines of Rust source that reference `needle` as **code**, one-based.
///
/// Comments are dropped, and so is everything from the first `#[cfg(test)]`
/// onward. Both exclusions are deliberate and both are approximations:
///
/// - A constant named in a comment is documentation, and the rule this serves is
///   about what the compiler links, not about what a reader is told.
/// - Truncating at `#[cfg(test)]` assumes the test module sits at the bottom of
///   the file, which is this repository's idiom without exception. A test module
///   in the middle would hide real code below it — so the cost of being wrong is
///   a check that under-reports, which is the direction a heuristic in a
///   conformance test should fail in. It never invents a violation.
///
/// It does not parse Rust. A `needle` inside a string literal counts, which is
/// the same direction: over-reporting is visible and under-reporting is not.
#[must_use]
pub fn code_references(source: &str, needle: &str) -> Vec<usize> {
    source
        .lines()
        .take_while(|line| !line.trim_start().starts_with("#[cfg(test)]"))
        .enumerate()
        .filter(|(_, line)| {
            let code = line.split("//").next().unwrap_or("");
            code.contains(needle)
        })
        .map(|(i, _)| i + 1)
        .collect()
}

/// Every tracked Rust source file under a crate's `src`, excluding `excluding`.
///
/// `tests/` directories are absent by construction: a test may legitimately
/// name anything, and the rules built on this are about production code.
#[must_use]
pub fn crate_sources(excluding: &str) -> Vec<String> {
    known_files()
        .into_iter()
        .filter(|f| f.starts_with("crates/") && f.contains("/src/"))
        .filter(|f| {
            // `Path::extension` rather than `ends_with(".rs")`: the string form
            // is a case-sensitive comparison, and this repository is developed
            // on a case-insensitive filesystem where `.RS` would be the same
            // file and would silently escape the rule.
            Path::new(f)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("rs"))
        })
        .filter(|f| !f.starts_with(excluding))
        .collect()
}

/// Whether `crate_name` lists `dependency` outside `[dev-dependencies]`.
///
/// The distinction is load-bearing rather than pedantic. `radar-pumpfun`
/// depends on `radar-signer` under `[dev-dependencies]` so that the signer can
/// check what the builder produces; a rule that read the whole manifest would
/// forbid the very test that enforces rule 1. What ships is what sits above
/// that header.
#[must_use]
pub fn production_dependency(crate_name: &str, dependency: &str) -> bool {
    let manifest = root().join("crates").join(crate_name).join("Cargo.toml");
    std::fs::read_to_string(&manifest).is_ok_and(|text| {
        text.lines()
            .take_while(|l| !l.trim_start().starts_with("[dev-dependencies]"))
            .any(|l| l.split('#').next().unwrap_or("").contains(dependency))
    })
}

/// Markdown files present in the working tree that `git` does not know about.
///
/// Every check in this crate reads [`known_files`], which is `git ls-files`. A
/// markdown document is therefore checked by *nothing* until it is added to the
/// index -- its links are not resolved, the paths it names are not verified, its
/// status field is not required. This was found by verifying design `0004`,
/// which passed only once it had been `git add -N`'d, and it is the same hole
/// the justfile already closes for Rust.
///
/// `--exclude-standard` means `.gitignore` is the escape hatch: a scratch file
/// that should not be checked should say so there.
#[must_use]
pub fn untracked_documents() -> Vec<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root())
        .args(["ls-files", "--others", "--exclude-standard"])
        .output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    parse_ls_files(&String::from_utf8_lossy(&out.stdout))
        .into_iter()
        .filter(|f| {
            Path::new(f.as_str())
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        })
        .collect()
}

/// Parses `.github/required-checks.txt` into `(context, command)` pairs.
///
/// Left of the first `=` is the status-check context exactly as the branch
/// ruleset names it; right of it is how the job runs. Comments and blank lines
/// are dropped. Split out from the test because a parser can be tested and a
/// file read cannot.
#[must_use]
pub fn required_checks(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(c, r)| (c.trim().to_string(), r.trim().to_string()))
        .collect()
}

/// Recipe names defined in the justfile.
///
/// A recipe is a line beginning at column zero with a name and a colon. Two
/// shapes have to be told apart from it, and the first version of this function
/// got both wrong:
///
/// - **`name := value` is a variable**, not a recipe. `cargo := env(...)` read
///   as a recipe called `cargo`.
/// - **A recipe may take parameters**, and they sit between the name and the
///   colon: `mutants base="origin/main" shard="":`. Taking everything before
///   the colon read that as a recipe nobody could ever have named, so the check
///   reported a real recipe as missing.
///
/// Recipes starting with `_` are internal and are not checks.
#[must_use]
pub fn justfile_recipes(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter(|l| !l.starts_with([' ', '\t', '#']))
        .filter_map(|l| {
            let (head, rest) = l.split_once(':')?;
            if rest.starts_with('=') {
                return None;
            }
            head.split_whitespace().next()
        })
        .filter(|name| {
            !name.is_empty()
                && !name.starts_with('_')
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
        .map(str::to_string)
        .collect()
}

/// Status-check contexts a workflow can actually produce.
///
/// Two sources, because the workflow has two shapes. A job's `name:` at four
/// spaces of indentation is a context directly; a step's `- name:` is not, and
/// the indentation is what tells them apart. A `matrix.recipe` list fans one
/// job out into one context per entry, which is where five of the nine come
/// from.
#[must_use]
pub fn workflow_contexts(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("    name: ") {
            let name = name.trim();
            // A templated job name names no context on its own; the matrix
            // below it does.
            if !name.contains(TEMPLATE_OPEN) {
                out.insert(name.to_string());
            }
        }
        if let Some(list) = line.trim().strip_prefix("recipe: [")
            && let Some(list) = list.strip_suffix(']')
        {
            out.extend(list.split(',').map(|r| r.trim().to_string()));
        }
    }
    out
}

/// Shell lines a workflow actually runs, with comments and YAML keys dropped.
///
/// Deliberately crude, and crude in the safe direction. It keeps every line
/// inside a `run:` block and drops anything after a `#`, so a rule built on it
/// reads what a runner would execute rather than what a comment says about it.
/// A `#` inside a quoted shell string would be dropped too; that under-reports,
/// which is the direction a heuristic in a conformance check should fail in.
#[must_use]
pub fn workflow_run_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut indent = None;
    for line in text.lines() {
        let depth = line.len() - line.trim_start().len();
        let trimmed = line.trim();

        // A `run:` on one line carries its command with it; a `run: |` opens a
        // block that continues while the indentation stays deeper than the key.
        //
        // The `- ` is stripped first because a `run:` is usually the first key
        // of a list item and is written `- run:`. Missing that made the first
        // version of this find nothing at all, which the test below caught.
        let key = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        if let Some(rest) = key.strip_prefix("run:") {
            let rest = rest.trim();
            indent = Some(depth);
            if !rest.is_empty() && rest != "|" && rest != ">" {
                out.push(rest.to_string());
            }
            continue;
        }
        match indent {
            Some(_) if trimmed.is_empty() => {}
            Some(open) if depth > open => {
                let code = trimmed.split('#').next().unwrap_or("").trim();
                if !code.is_empty() {
                    out.push(code.to_string());
                }
            }
            Some(_) => indent = None,
            None => {}
        }
    }
    out
}

/// The opening of a GitHub Actions expression, spelled out so this file does
/// not contain a literal one for a workflow linter to trip over.
const TEMPLATE_OPEN: &str = "${{";

/// Integration-test paths named anywhere in a document.
///
/// Normalised so that `../crates/x/tests/y.rs` from inside `docs/` and
/// `crates/x/tests/y.rs` from the root are recognised as the same file --
/// otherwise the rule built on this would pass precisely when the collision is
/// between a document in `docs/` and one at the root, which is the common case.
#[must_use]
pub fn test_paths(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes: Vec<char> = text.chars().collect();
    let needle: Vec<char> = "/tests/".chars().collect();
    for start in 0..bytes.len() {
        if !bytes[start..].starts_with(needle.as_slice()) {
            continue;
        }
        let is_path = |c: &char| c.is_ascii_alphanumeric() || matches!(*c, '_' | '-' | '.' | '/');

        // The run of path characters containing this match, found by searching
        // rather than by walking an index in a loop.
        //
        // The loops this replaces were `from -= 1` and `to += 1`, and CI
        // reported both as timeouts: mutated to `/=` and `*=` the counters stop
        // moving and the scan never ends, so each one cost five minutes and
        // failed the job. A hang is a poor way to detect a bug, and a search
        // cannot hang at all -- which is cheaper than a test that catches it
        // after 300 seconds.
        let from = bytes[..start]
            .iter()
            .rposition(|c| !is_path(c))
            .map_or(0, |i| i + 1);
        let to = start
            + bytes[start..]
                .iter()
                .position(|c| !is_path(c))
                .unwrap_or(bytes.len() - start);
        let path: String = bytes[from..to].iter().collect();
        if let Some(path) = path.strip_suffix(".rs") {
            let path = path.trim_start_matches("../");
            out.insert(format!("{path}.rs"));
        }
    }
    out
}

/// The numbered entries in `LEARNINGS.md`, as `(number, heading)`.
#[must_use]
pub fn learnings_entries(text: &str) -> Vec<(u32, String)> {
    text.lines()
        .filter_map(|l| l.strip_prefix("## "))
        .filter_map(|h| {
            let (number, _) = h.split_once(". ")?;
            Some((number.parse().ok()?, h.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

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
    fn nothing_that_listens_on_a_network_depends_on_the_signer_crate() {
        // AGENTS.md rule 1 says the signer "has no network, no listener and no
        // method that signs arbitrary bytes". That was true of the signer
        // *binary* and not of the signer *crate*: `radar-serve` — the
        // internet-facing HTTP server — listed `radar-signer` as a dependency,
        // so `Key::load` and `Key::sign` were compiled into its address space.
        //
        // Nothing was called. That is exactly why it survived review, and why
        // asserting it is worth more than fixing it: the only thing the web
        // server ever wanted was a base64 codec, which now lives in
        // `radar-types` where every crate can reach it without reaching past it.
        //
        // The rule is about which processes can hold a key, so it is checked
        // against the crates that bind a socket rather than against everything.
        // `radar-agent` is here for a different reason and the same rule. It
        // does not listen, but it is the boundary a *model* sits behind, and
        // model judgement must never authorise capital. A path from there to the
        // signer is the exact thing rule 1 forbids -- and it would arrive as a
        // convenience, one `use` at a time, the way the last one did.
        // `radar-model` joins them for the third reason: it is the only crate
        // that holds a credential, and a crate holding a credential must not
        // also be able to reach a key.
        const LISTENERS: &[&str] = &["radar-serve", "radar-agent", "radar-model"];

        for listener in LISTENERS {
            let manifest = root().join("crates").join(listener).join("Cargo.toml");
            let text = std::fs::read_to_string(&manifest)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
            assert!(
                !text.contains("radar-signer"),
                "{listener} listens on a network and must not depend on radar-signer; \
                 if it needs something from there, that something belongs somewhere else"
            );
        }

        // And neither AI crate reaches anything that could become an
        // authorisation. "No path" has to mean no path: a crate able to build a
        // `Proposal` or reach `radar-exec` is one refactor from authorising one,
        // and the dependency would arrive as a convenience the way the last one
        // did.
        for crate_name in ["radar-agent", "radar-model"] {
            let manifest = root().join("crates").join(crate_name).join("Cargo.toml");
            let text = std::fs::read_to_string(&manifest)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
            for forbidden in ["radar-risk", "radar-exec", "radar-strategy", "radar-store"] {
                assert!(
                    !text.contains(forbidden),
                    "{crate_name} must not depend on {forbidden}: a model that can reach                      the decision lane is a model that can reach capital, whatever the                      intention was when the dependency was added"
                );
            }
        }
    }

    #[test]
    fn the_documented_dependency_claims_are_true() {
        const NOT_REACHABLE: &[&str] = &[
            "radar_exec::pipeline::execute",
            "radar_exec::signer_client",
            "radar_exec::submit",
            "radar_exec::customer_signing",
        ];

        // `README.md` and `docs/STATE.md` each carried the sentence "No
        // production crate depends on `radar-exec`; the composition reaches it
        // through a dev-dependency" from 2026-09-03. It was false when it was
        // written: `radar-cli` had listed `radar-exec` under `[dependencies]`
        // since 2026-08-31, for `radar route`.
        //
        // The sentence mattered more than most, because it is the one that says
        // the shipped dependency graph cannot reach the trading path — the
        // property AGENTS.md rule 1 exists to hold. A claim of that shape is
        // exactly what this crate is for, so it is pinned rather than merely
        // corrected. Three rows, deliberately: enough to make the prose
        // falsifiable, narrow enough that ordinary manifest edits do not trip
        // it. When one of them does trip, the fix is to re-read the paragraphs
        // it names before editing the row.

        // Row 1 — exactly one crate outside `radar-exec` depends on it.
        // The documents may say "one production caller, and it is the router";
        // they may not say "none".
        let dependents: BTreeSet<String> = crate_directories()
            .into_iter()
            .filter(|c| c != "radar-exec")
            .filter(|c| production_dependency(c, "radar-exec"))
            .collect();
        let expected: BTreeSet<String> = ["radar-cli".to_string()].into_iter().collect();
        assert_eq!(
            dependents, expected,
            "the set of production crates depending on radar-exec changed. \
             README.md and docs/STATE.md both describe this set in prose; \
             re-read those paragraphs, correct them, then update this row"
        );

        // Row 2 — and that one caller reaches the router only. `execute` is the
        // function that signs and sends; the crates that lead to it are its
        // siblings. A production `use` of any of them is the trading path
        // acquiring a caller, which is a decision about money and not a wiring
        // change.
        for source in crate_sources("crates/radar-exec/") {
            let text = std::fs::read_to_string(root().join(&source))
                .unwrap_or_else(|e| panic!("cannot read {source}: {e}"));
            for symbol in NOT_REACHABLE {
                assert!(
                    code_references(&text, symbol).is_empty(),
                    "{source} names {symbol}. Production code reaching the send \
                     path is what README.md's \"there is no production caller \
                     for the trading path\" denies -- correct the documents \
                     first, or do not add the call"
                );
            }
        }

        // Row 2b -- the other pinned edge, and the reason it is here rather
        // than in a rule of its own: docs/STATE.md describes `radar-provider` as
        // the crate `radar-agent` meters through, and that sentence has already
        // been stale once. Design 0004 §5 named this edge alongside the other
        // two.
        let provider_dependents: BTreeSet<String> = crate_directories()
            .into_iter()
            .filter(|c| c != "radar-provider")
            .filter(|c| production_dependency(c, "radar-provider"))
            .collect();
        assert!(
            provider_dependents.contains("radar-agent"),
            "docs/STATE.md says radar-agent meters through radar-provider and the manifest does not agree; production dependents of radar-provider are {provider_dependents:?}"
        );

        // Row 3 — nothing outside `radar-exec` binds a key. Rows 1 and 2 are
        // about a graph and a symbol; this one is about the sentence's actual
        // subject, which is that no shipped process can sign. It is cheap and
        // it fails for a reason the other two would miss -- a new crate that
        // depends on `radar-signer` directly rather than through the executor.
        //
        // It is a *production* dependency that is forbidden, not any mention.
        // `radar-pumpfun` lists `radar-signer` under `[dev-dependencies]` on
        // purpose and says why in its manifest: the builder is checked by the
        // signer, and the direction is the point. A rule that read the whole
        // file would forbid the test that enforces rule 1.
        for c in crate_directories() {
            if c == "radar-exec" || c == "radar-signer" {
                continue;
            }
            assert!(
                !production_dependency(&c, "radar-signer"),
                "{c} depends on radar-signer outside [dev-dependencies]; only radar-exec may, and only through the signer process (AGENTS.md rule 1)"
            );
        }
    }

    #[test]
    fn the_context_file_stays_within_its_budget() {
        // `AGENTS.md` is loaded on every turn of every task in this repository,
        // and instructions in a file like it are followed at well above baseline
        // rates -- so a line that is merely unnecessary is obeyed rather than
        // ignored, and billed. Gloaguen et al. (MemAgents @ ICLR 2026) measured
        // no task-success improvement from repository context files against a
        // cost over 20% higher. See
        // `docs/research/0025-what-the-evidence-says-about-how-this-repository-is-run.md`
        // §1.
        //
        // That finding was prose until this test existed, which meant the file
        // could drift back. It went 519 -> 361 on 2026-09-03 by moving status to
        // `docs/STATE.md`; the ceiling exists so the next fifty lines of status
        // have to be argued for rather than merely added.
        //
        // **This is a budget, not a target.** Being under it is not a reason to
        // add anything, and the right way to come back under it is almost always
        // to move something to STATE.md, LEARNINGS.md or a check -- not to
        // compress a rule until it misleads. Rules 1 and 3 are long on purpose:
        // both record that an earlier, terser version of themselves was read as
        // claiming more than it did.
        //
        // If a genuine new rule needs the room, raise the number in the same
        // commit and say why. The number moving is the signal; a file quietly
        // growing is what this prevents.
        const CEILING: usize = 400;

        let text = std::fs::read_to_string(root().join("AGENTS.md")).expect("AGENTS.md");
        let lines = text.lines().count();
        assert!(
            lines <= CEILING,
            "AGENTS.md is {lines} lines against a ceiling of {CEILING}. Move status to docs/STATE.md, war stories to LEARNINGS.md, and anything a check already enforces to a pointer -- or raise the ceiling in this commit and say what rule needed the room"
        );

        // And the other direction, because a ceiling alone is satisfied by an
        // empty file. This is not a quality measure -- it catches the file being
        // truncated or emptied, which is how it was lost once already
        // (2026-09-02, a careless `git add -A`).
        assert!(
            lines > 150,
            "AGENTS.md is only {lines} lines, which is too few to still contain rules 1 through 9. It has been deleted by accident before"
        );

        // `CLAUDE.md` imports it rather than restating it. Two files with the
        // same content is two files that drift, and this repository has the
        // comment in `CLAUDE.md` saying exactly that.
        let claude = std::fs::read_to_string(root().join("CLAUDE.md")).expect("CLAUDE.md");
        assert!(
            claude.contains("@AGENTS.md"),
            "CLAUDE.md must import AGENTS.md rather than carry its own copy of the policy"
        );
        assert!(
            claude.lines().count() < 20,
            "CLAUDE.md is {} lines. It is an import, not a second policy file",
            claude.lines().count()
        );
    }

    #[test]
    fn every_learnings_entry_names_what_catches_a_recurrence_and_is_indexed() {
        // The file opens by promising that "each entry names the check that
        // would catch a recurrence, or says plainly that nothing does". Design
        // 0004 §3.2 measured the promise: entries 1 to 19 kept it, 20 to 28 used
        // a different header for a weaker thing, and five kept it in neither
        // form. A standard the file states about itself and does not hold is
        // the same defect as a document describing code that changed.
        //
        // The header is one spelling on purpose. "Nothing mechanical, the habit
        // is X" is a valid answer -- eight entries give it -- and it is only
        // legible as an answer when it appears in the same place as the others.
        const HEADER: &str = "**What catches a recurrence:**";

        let text = std::fs::read_to_string(root().join("LEARNINGS.md")).expect("LEARNINGS.md");
        let entries = learnings_entries(&text);
        assert!(
            entries.len() > 20,
            "LEARNINGS.md parsed to {} entries, which is too few to be right and would make the assertions below vacuous",
            entries.len()
        );

        // Split once, on the headings, so each entry is checked against its own
        // body rather than against the whole file.
        let mut sections: Vec<(u32, String, String)> = Vec::new();
        let mut current: Option<(u32, String, String)> = None;
        for line in text.lines() {
            if let Some(heading) = line.strip_prefix("## ")
                && let Some((number, _)) = heading.split_once(". ")
                && let Ok(number) = number.parse::<u32>()
            {
                if let Some(done) = current.take() {
                    sections.push(done);
                }
                current = Some((number, heading.to_string(), String::new()));
                continue;
            }
            if let Some((_, _, body)) = current.as_mut() {
                body.push_str(line);
                body.push('\n');
            }
        }
        if let Some(done) = current.take() {
            sections.push(done);
        }

        let mut missing = Vec::new();
        let mut unindexed = Vec::new();
        for (number, heading, body) in &sections {
            if !body.contains(HEADER) {
                missing.push(heading.clone());
            }
            // The index links each entry by number. A new entry with no row is
            // not visibly absent from a table of twenty-eight, which is exactly
            // how a navigation aid stops being one.
            if !text.contains(&format!("| [{number}](#")) {
                unindexed.push(*number);
            }
        }

        assert!(
            missing.is_empty(),
            "these LEARNINGS entries do not say what catches a recurrence: {missing:?}. The file's own opening requires it, and {HEADER:?} is the spelling -- `nothing mechanical, the habit is ...` is a valid answer and eight entries give it"
        );
        assert!(
            unindexed.is_empty(),
            "these LEARNINGS entries have no row in the index table: {unindexed:?}. An entry missing from a table of {} is not visibly missing, which is how a navigation aid quietly stops being one",
            sections.len()
        );
    }

    #[test]
    fn a_test_path_is_read_to_its_edges_and_no_further() {
        // `test_paths` had no direct test: it was exercised only through the
        // ownership rule, which passes trivially when there are no collisions --
        // so a suite with zero collisions could not tell a working extractor
        // from one that returned nothing. CI reported nine survivors here.
        let one = |t: &str| test_paths(t).into_iter().collect::<Vec<_>>();

        // A path that is the whole string. The backward walk has to stop at
        // index 0 without reading before it, and the forward walk has to stop at
        // the end without reading past it.
        assert_eq!(one("crates/a/tests/b.rs"), vec!["crates/a/tests/b.rs"]);

        // Surrounded by characters that are not part of a path -- a markdown
        // link is the real case. Without a correct backward walk the opening
        // bracket is swallowed into the path.
        assert_eq!(
            one("see [b](crates/a/tests/b.rs) for it"),
            vec!["crates/a/tests/b.rs"]
        );

        // Relative from inside `docs/`, normalised to the same path. This is the
        // whole reason the rule works between a root document and a docs/ one.
        assert_eq!(one("../crates/a/tests/b.rs"), vec!["crates/a/tests/b.rs"]);

        // Two in one line, and a `/tests/` path that is not Rust.
        assert_eq!(
            one("crates/a/tests/x.rs and crates/b/tests/y.rs"),
            vec!["crates/a/tests/x.rs", "crates/b/tests/y.rs"]
        );
        assert!(one("crates/a/tests/fixture.json").is_empty());

        // A Rust file that is not a test file. This is the case that pins the
        // `/tests/` filter itself: without it, inverting the filter still finds
        // every path, because the backward walk from any earlier position
        // expands to the same string. The rule is about test files, and a `src`
        // path must not be read as one.
        assert!(one("crates/a/src/b.rs").is_empty());
        assert!(one("see [x](crates/a/src/b.rs) and nothing else").is_empty());

        // Prose with no path in it at all.
        assert!(one("the tests directory").is_empty());
        assert!(one("").is_empty());
    }

    #[test]
    fn one_test_file_is_accounted_for_by_one_document() {
        // Three documents make claims about behaviour: AGENTS.md says what the
        // rules are, README.md is the front page, docs/STATE.md is what has
        // actually been built. When two of them name the same test file, two of
        // them are describing the same evidence -- and design 0004 §3.1 is the
        // case for why that matters: the sentence about radar-exec was wrong in
        // both README.md and docs/STATE.md, because it had been written twice
        // and corrected in neither.
        //
        // The rule is ownership, not silence. A document that wants to mention
        // a test links the document that owns the account instead, which is what
        // README.md now does for the composition tests.
        const CLAIMANTS: &[&str] = &["AGENTS.md", "README.md", "docs/STATE.md"];

        let mut owner: BTreeMap<String, &str> = BTreeMap::new();
        let mut collisions: Vec<String> = Vec::new();
        for document in CLAIMANTS {
            let text = std::fs::read_to_string(root().join(document))
                .unwrap_or_else(|e| panic!("cannot read {document}: {e}"));
            for path in test_paths(&text) {
                if let Some(first) = owner.insert(path.clone(), document) {
                    collisions.push(format!("{path} in both {first} and {document}"));
                }
            }
        }
        assert!(
            collisions.is_empty(),
            "these test files are accounted for by more than one document: {collisions:?} -- pick the one that owns the account and have the other link that document instead. Two documents describing one test is how the same sentence gets corrected in one place and not the other"
        );
    }

    #[test]
    fn no_workflow_runs_the_node_toolchain_itself() {
        // On 2026-09-04 `just web` was taught to retry `npm audit` when the
        // registry's advisory endpoint is unreachable -- a vulnerability and an
        // unreachable registry are different answers, and the exit code alone
        // does not tell them apart. The `web` job went green with the fix. The
        // release job failed eight minutes later, on a 503 from that same
        // endpoint, because it held its own inline copy of the three commands
        // and the fix had landed in the recipe.
        //
        // So the rule is narrow and it is exactly the failure: the Node
        // toolchain has one definition, in the `web` recipe, and a workflow
        // reaches it through `just`. Anything a future job needs from npm is a
        // change to that recipe, which is also what makes this check cheap --
        // there is no reasonable change it fires on.
        //
        // `cargo` is deliberately NOT included. `release-linux` builds the
        // binaries directly and should: that is a release artifact rather than
        // a check, and no recipe owns it.
        let dir = root().join(".github/workflows");
        let mut offenders = Vec::new();
        let mut seen = 0;
        for entry in std::fs::read_dir(&dir).expect(".github/workflows must be readable") {
            let path = entry.expect("a readable directory entry").path();
            if path.extension().is_none_or(|e| e != "yml") {
                continue;
            }
            seen += 1;
            let text = std::fs::read_to_string(&path).expect("a readable workflow");
            for line in workflow_run_lines(&text) {
                if line.split_whitespace().next() == Some("npm") {
                    offenders.push(format!("{}: {line}", path.display()));
                }
            }
        }
        // Without this the check passes when the directory is empty, misread or
        // renamed -- which is LEARNINGS 5's shape, an absent answer reported the
        // same way as a clean one.
        assert!(
            seen > 0,
            "no workflows were read; the check would pass vacuously"
        );
        assert!(
            offenders.is_empty(),
            "a workflow runs npm directly instead of `just web`: {}. The recipe owns the Node toolchain, and a second copy is a fix that lands in only one of them.",
            offenders.join(", ")
        );
    }

    #[test]
    fn a_run_block_is_read_as_commands_and_a_comment_is_not_one() {
        // The rule above is only as good as this: a comment that *mentions* the
        // command it forbids must not be read as running it, and the comment
        // written beside that fix does mention it.
        let one = |t: &str| workflow_run_lines(t);

        assert_eq!(one("      - run: just web"), vec!["just web"]);
        assert_eq!(
            one("      - run: |
          npm ci
          npm run build
"),
            vec!["npm ci", "npm run build"]
        );
        // A comment inside the block, and a trailing comment on a real command.
        assert_eq!(
            one("      - run: |
          # npm ci is what this replaced
          just web # not npm
"),
            vec!["just web"]
        );
        // A comment *outside* any run block, which is where the explanation for
        // the fix actually lives.
        assert!(
            one("      # this step used to run npm ci inline
      - uses: actions/checkout@v5")
            .is_empty()
        );
        // The block ends when the indentation returns to the key's level.
        assert_eq!(
            one("      - run: |
          npm ci
      - uses: actions/checkout@v5
"),
            vec!["npm ci"]
        );
        assert!(one("").is_empty());
    }

    #[test]
    fn the_required_checks_file_agrees_with_the_justfile_and_the_workflow() {
        // The file says it itself: "Adding a check means adding it in three
        // places: the ruleset, a workflow job, and here." Two of those three are
        // in this repository and can be compared. The ruleset is not, which is
        // why the file writes the `gh api` query rather than the answer -- and
        // why LEARNINGS 13 happened, where the document describing the
        // enforcement was the only evidence the enforcement existed.
        //
        // So this checks the half that is checkable: every context the file
        // requires is one a workflow can actually produce, and every command it
        // names is a recipe that exists.
        let checks = std::fs::read_to_string(root().join(".github/required-checks.txt"))
            .expect("required-checks.txt");
        let justfile = std::fs::read_to_string(root().join("justfile")).expect("justfile");
        let workflow =
            std::fs::read_to_string(root().join(".github/workflows/ci.yml")).expect("ci.yml");

        let checks = required_checks(&checks);
        assert!(
            !checks.is_empty(),
            "required-checks.txt parsed to nothing, which would make every assertion below vacuous"
        );
        let recipes = justfile_recipes(&justfile);
        let contexts = workflow_contexts(&workflow);

        for (context, command) in &checks {
            assert!(
                contexts.contains(context),
                "required-checks.txt requires the context {context:?}, and no job in ci.yml produces it. A required check no workflow reports is a check that never turns green. Produced: {contexts:?}"
            );

            // `github-only:` is the escape hatch and it is used once, for msrv,
            // whose toolchain the host may not have. It is spelled out rather
            // than inferred so that adding a second one is a deliberate act.
            if command.starts_with("github-only:") {
                continue;
            }
            let recipe = command.strip_prefix("just ").unwrap_or_else(|| {
                panic!(
                    "required-checks.txt maps {context:?} to {command:?}, which is neither `just <recipe>` nor `github-only: <reason>`. The point of the file is that the workflow holds no copy of the command"
                )
            });
            assert!(
                recipes.contains(recipe),
                "required-checks.txt maps {context:?} to `just {recipe}`, and the justfile has no such recipe. Recipes: {recipes:?}"
            );
        }

        // And the other direction, for the fan-out only. A job may exist without
        // being required -- `mutants-shards` is one, deliberately, since
        // `mutants` gates on it. But a recipe added to the `check` matrix is a
        // check somebody meant to run, and one that runs while gating nothing is
        // the quieter failure the file's own comment records about `web`.
        let required: BTreeSet<&str> = checks.iter().map(|(c, _)| c.as_str()).collect();
        for line in workflow.lines() {
            if let Some(list) = line.trim().strip_prefix("recipe: [")
                && let Some(list) = list.strip_suffix(']')
            {
                for recipe in list.split(',').map(str::trim) {
                    assert!(
                        required.contains(recipe),
                        "ci.yml runs {recipe:?} in the check matrix and required-checks.txt does not require it. A check that runs and gates nothing looks like a gate and is not one -- require it, or take it out of the matrix"
                    );
                }
            }
        }
    }

    #[test]
    fn the_required_checks_parsers_read_what_the_files_actually_say() {
        // Three parsers, three ways to be silently empty -- and an empty result
        // makes the assertions above pass for any repository at all. Each is
        // pinned against the shape it must not misread.
        let parsed = required_checks(
            "# a comment = not a check\n\n  build = just build\nmsrv = github-only: no local recipe\n",
        );
        assert_eq!(
            parsed,
            vec![
                ("build".to_string(), "just build".to_string()),
                (
                    "msrv".to_string(),
                    "github-only: no local recipe".to_string()
                ),
            ],
            "a comment containing `=` must not read as a check, and the value keeps its own colons"
        );

        // Every shape this has to tell apart, including the two that were wrong
        // on the first run: a variable assignment, and a recipe with parameters.
        // `Web:` is here for the third conjunct in the name filter -- a name
        // outside the allowed character set is not a recipe name, and without
        // that clause an arbitrary capitalised line reads as one.
        let recipes = justfile_recipes(concat!(
            "cargo := env(\"RADAR_CARGO\", \"cargo\")\n",
            "check: _disk build\n",
            "    cargo test --cfg a:b\n",
            "_disk:\n",
            "Web:\n",
            "mutants base=\"origin/main\" shard=\"\":\n",
            "web:\n",
        ));
        assert_eq!(
            recipes,
            ["check", "mutants", "web"]
                .into_iter()
                .map(str::to_string)
                .collect::<BTreeSet<_>>(),
            "`:=` is a variable, a parameterised recipe is named by its first word, an indented body line is not a recipe, an underscore recipe is not a check, and `Web` is not a recipe name this repository can have"
        );

        let templated = format!("    name: {TEMPLATE_OPEN} matrix.recipe }}}}");
        let workflow = format!(
            "  check:\n{templated}\n        recipe: [build, tests]\n  web:\n    name: web\n    steps:\n      - name: every shard passed\n"
        );
        assert_eq!(
            workflow_contexts(&workflow),
            ["build".to_string(), "tests".to_string(), "web".to_string()]
                .into_iter()
                .collect::<BTreeSet<_>>(),
            "a step name is not a context and a templated job name names none by itself"
        );
    }

    #[test]
    fn no_markdown_document_is_invisible_to_these_checks() {
        // Declared first: an item after a statement is a clippy warning, and CI
        // builds with `-D warnings`.
        struct Probe(PathBuf);
        impl Drop for Probe {
            fn drop(&mut self) {
                // Best effort on purpose: a failed assertion must not be
                // replaced by a panic-in-drop about the cleanup.
                let _ = std::fs::remove_file(&self.0);
            }
        }

        // The hole this closes is not hypothetical and was not found by
        // reasoning: design 0004 was written, run against this suite, and
        // passed -- because the suite could not see it. It passed for real only
        // after `git add -N`. Everything in this crate reads `git ls-files`, so
        // an unstaged document is checked by nothing at the moment when its
        // links and its claims are most likely to be wrong.
        //
        // The justfile already guards the identical hole for Rust. This is that
        // guard for prose.
        let untracked = untracked_documents();
        assert!(
            untracked.is_empty(),
            "these markdown files exist but git does not know about them, so every check in this crate silently skipped them: {untracked:?} -- run `git add -N <path>` to make them visible, or add them to .gitignore to say they are not documents"
        );

        // And the other half, in the same test rather than its own, because a
        // second test creating an untracked file would race the assertion above.
        //
        // The assertion above is satisfied by a function that always returns
        // nothing, which is exactly what CI reported: two survivors that gutted
        // `untracked_documents` and nothing failed. A rule that cannot tell "no
        // untracked documents" from "I did not look" is LEARNINGS 5, and this
        // crate exists to catch that shape.
        // Skipped where there is no git directory to ask -- under `cargo
        // mutants`, which works in a copy of the sources without one. There
        // `untracked_documents` is dead whatever its body says, so failing here
        // would be failing for the environment. `.cargo/mutants.toml` records
        // the two mutants that consequently survive a mutation run, as
        // unreachable in that sandbox rather than as equivalent: they are not
        // equivalent, they disable the rule.
        //
        // Written inline rather than as a `git_is_usable()` helper, which was
        // the first attempt: a function whose only job is to gate a test is a
        // function whose mutation to `false` skips the test and can never be
        // killed. Introducing an unkillable mutant to kill two others is a bad
        // trade.
        if !root().join(".git").exists() {
            return;
        }

        let probe = Probe(root().join("zz-untracked-probe.md"));
        std::fs::write(&probe.0, "# not a document, a probe\n").expect("write the probe");
        let seen = untracked_documents();
        assert!(
            seen.iter().any(|f| f.ends_with("zz-untracked-probe.md")),
            "untracked_documents did not report a markdown file that is present and untracked, so the assertion above proves nothing. It saw: {seen:?}"
        );
    }

    #[test]
    fn every_numbered_document_declares_a_status() {
        // Design 0004 measured this convention at 38 of 48: every ADR carried
        // `**Status:**` and ten research notes did not -- and the ten were the
        // oldest, which is to say the ones most likely to have been overtaken.
        // A reader cannot tell an old note that still holds from one that was
        // corrected two months ago, and the notes that were corrected are
        // exactly the ones a fresh session is most likely to reason from.
        //
        // The field is deliberately free text rather than an enum. In
        // `docs/adr/` it says accepted or superseded; in `docs/research/` it
        // says how strongly the thing was measured, which is a sentence and not
        // a keyword. What is checkable is that the question was answered at all,
        // near the top, where somebody skimming will see it.
        const HEADER_LINES: usize = 15;

        let mut missing = Vec::new();
        for dir in ["docs/adr", "docs/design", "docs/research"] {
            let path = root().join(dir);
            let entries = std::fs::read_dir(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            for entry in entries {
                let file = entry.expect("directory entry").path();
                if file.extension().is_none_or(|e| e != "md") {
                    continue;
                }
                // `README.md` describes a directory rather than making a claim
                // in it, so there is nothing for a status to be about.
                if file.file_name().is_some_and(|n| n == "README.md") {
                    continue;
                }
                let text = std::fs::read_to_string(&file)
                    .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
                if !text
                    .lines()
                    .take(HEADER_LINES)
                    .any(|l| l.contains("**Status:**"))
                {
                    missing.push(format!("{dir}/{}", file.file_name().unwrap().display()));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "these documents do not declare a status in their first {HEADER_LINES} lines: {missing:?} -- a numbered document without one cannot be told apart from a superseded one, and the oldest are the likeliest to have been overtaken; say what it is worth now, in a sentence, not a keyword"
        );
    }

    #[test]
    fn every_file_path_named_in_the_documentation_exists_and_is_tracked() {
        // LEARNINGS entry 1 has said "nothing catches a recurrence" since the
        // day it was written. The failure it records is a sibling repository
        // documenting a design as canonical while citing a source file that is
        // in no commit — surviving only as stale build output.
        //
        // Relative *links* are already checked. This checks paths named in prose
        // and in code spans, which is how that repository's claim was written and
        // is the form a link checker cannot see.
        // Two levels, because this has to hold in two places. In a checkout,
        // git decides: the failure being guarded against is a file that exists
        // on somebody's disk and in no commit. Under `cargo mutants` the tree is
        // copied without a `.git`, so git can decide nothing -- and the honest
        // response is to fall back to existence on disk rather than to skip,
        // which would make the whole check vacuous exactly where it is least
        // observed.
        let known = known_files();
        assert!(
            !known.is_empty(),
            "no files found at all; the check would pass vacuously"
        );

        // Matched by suffix, because a document names a path the way a reader
        // would follow it -- `pipeline.rs` from prose about that file, or
        // `radar-store/tests/watermark_holds.rs` from a paragraph already inside
        // `crates/`. Requiring a repository-root path would flag correct prose,
        // and a check that flags correct prose is a check somebody turns off.
        let resolves = |named: &str| {
            known
                .iter()
                .any(|t| t == named || t.ends_with(&format!("/{named}")))
        };

        let mut missing = Vec::new();
        for doc in documents() {
            let Ok(text) = std::fs::read_to_string(&doc) else {
                continue;
            };
            for path in code_span_paths(&text) {
                if !resolves(&path) {
                    missing.push(format!("{} names {path}", doc.display()));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "documentation names files that are not tracked in git:\n  {}",
            missing.join("\n  ")
        );
    }

    #[test]
    fn ls_files_output_becomes_paths_and_never_an_empty_one() {
        // An empty entry is the dangerous one: `resolves` asks whether any known
        // path ends with `/{named}`, and "" would not match that — but a bare ""
        // in the set makes `is_empty()` false, so the fallback to walking the
        // tree never happens and the whole check runs against a set of nothing.
        let parsed = parse_ls_files("a/b.rs\n\n  c.toml  \n\n");
        assert_eq!(parsed.len(), 2, "blank lines are not paths: {parsed:?}");
        assert!(parsed.contains("a/b.rs"));
        assert!(parsed.contains("c.toml"), "surrounding space is trimmed");
        assert!(
            !parsed.contains(""),
            "an empty path matches nothing and hides that"
        );

        // Windows separators normalise, or every suffix match fails on Windows.
        assert!(
            parse_ls_files("crates\\radar-types\\src\\lib.rs")
                .contains("crates/radar-types/src/lib.rs")
        );

        // Nothing in, nothing out — which is what makes the caller fall back.
        assert!(parse_ls_files("").is_empty());
        assert!(parse_ls_files("\n\n").is_empty());
    }

    #[test]
    fn spans_are_paired_so_prose_between_them_is_not_read_as_one() {
        // Advancing past the closing backtick is what keeps spans paired. Off by
        // one and the *gaps* become spans, so `a.rs` followed by `b.rs` reads the
        // prose between them as a third — and the check starts reporting paths
        // nobody wrote.
        let found = code_span_paths("`a.rs` then not/a/path.rs then `b.rs`");
        assert_eq!(found.len(), 2, "exactly the two spans: {found:?}");
        assert!(
            found.contains("a.rs") && found.contains("b.rs"),
            "{found:?}"
        );
        assert!(
            !found.contains("not/a/path.rs"),
            "unquoted prose is not a code span: {found:?}"
        );
    }

    #[test]
    fn a_code_span_excludes_its_own_backticks() {
        // Off-by-one here is invisible in the happy path and fatal to the check:
        // a span read as "`Cargo.toml" resolves against nothing, so every named
        // path would be reported missing — or, read the other way, nothing is
        // extracted at all and the check passes for everything.
        let found = code_span_paths("see `Cargo.toml` here");
        assert_eq!(found.len(), 1, "{found:?}");
        let only = found.iter().next().expect("one");
        assert_eq!(
            only, "Cargo.toml",
            "the delimiters are not part of the path"
        );
        assert!(!only.starts_with('`') && !only.ends_with('`'));
    }

    #[test]
    fn the_path_extractor_finds_paths_and_ignores_prose() {
        // The check above is only worth having if this is right: too greedy and
        // it fails on ordinary backticked words, too narrow and it passes
        // vacuously — which is the failure mode it exists to prevent.
        let found = code_span_paths(
            "see `crates/radar-types/src/lib.rs` and `Cargo.toml`, but not `Slot` \
             or `foo.bar()` or `--flag` or `a/b`",
        );
        assert!(found.contains("crates/radar-types/src/lib.rs"), "{found:?}");
        assert!(found.contains("Cargo.toml"), "{found:?}");
        assert!(
            !found.contains("Slot"),
            "a type name is not a path: {found:?}"
        );
        assert!(
            !found.contains("foo.bar()"),
            "a call is not a path: {found:?}"
        );
        assert!(!found.contains("--flag"), "a flag is not a path: {found:?}");
        // A bare `a/b` with no extension is as likely to be prose as a path.
        assert!(!found.contains("a/b"), "{found:?}");

        // A path somewhere else is not a claim about this repository, and has to
        // be written so that both a reader and this check can tell.
        let external =
            code_span_paths("`claw-net:internal/x.md` and `/etc/systemd/system/y.service`");
        assert!(external.is_empty(), "{external:?}");
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

    #[test]
    fn the_embedded_interface_directory_is_tracked() {
        // `radar-serve` embeds `web/dist` with `rust-embed`, and the derive
        // generates no `get` method when the folder does not exist -- so a
        // checkout without it fails to compile, with an error naming a missing
        // *method* rather than a missing directory. `embed.rs` is written on the
        // assumption that an **empty** dist is normal; an **absent** one is not.
        //
        // The contents are build output and correctly ignored. The directory is
        // tracked through one empty file, and that file is easy to lose:
        // `emptyOutDir: true` deletes it on every build, so anyone who builds and
        // commits with `git add -A` stages the deletion. That happened, and it
        // turned five CI jobs red while every local build stayed green -- because
        // locally the directory was full.
        //
        // `vite.config.ts` recreates it after each build. This is the check that
        // the recreation is still working.
        assert!(
            known_files().contains("web/dist/.gitkeep"),
            "web/dist/.gitkeep is not tracked, so a fresh checkout has no              web/dist and radar-serve will not compile"
        );
    }

    #[test]
    fn only_radar_risk_names_the_closed_policy_in_code() {
        // The rule: `Policy::CLOSED` is a value, and every other crate must go
        // through `Policy::SHIPPED` -- the constant the decider uses -- so that
        // opening the policy moves everything that reports on it.
        //
        // This is a structural check because the alternative is discipline, and
        // discipline is what failed. Three call sites read `Policy::CLOSED`
        // independently: `radar consider`, which decides; `radar-serve`'s
        // funnel, which drove the banner "Nothing can be authorised"; and
        // `radar brief`'s `trading_lane`, which hardcoded `Status::Ok` beside
        // the words "no proposal can become an authorization". Opening the first
        // would have left the other two reassuring a reader while Radar traded.
        //
        // The third was found by writing this check rather than by reading the
        // code, which is the argument for having it.
        let mut offenders: Vec<String> = Vec::new();
        for file in crate_sources("crates/radar-risk/") {
            let Ok(text) = std::fs::read_to_string(root().join(&file)) else {
                continue;
            };
            for line in code_references(&text, "Policy::CLOSED") {
                offenders.push(format!("{file}:{line}"));
            }
        }
        assert!(
            offenders.is_empty(),
            "these name Policy::CLOSED in code; use Policy::SHIPPED so the              interface moves when the decider does: {offenders:?}"
        );
    }

    #[test]
    fn the_shipped_policy_is_actually_reached_from_outside_radar_risk() {
        // Guards the check above against passing vacuously. If nothing outside
        // `radar-risk` referenced a policy constant at all -- because the
        // extractor broke, or because `crate_sources` returned nothing -- the
        // assertion would hold while checking nothing.
        let reached: Vec<String> = crate_sources("crates/radar-risk/")
            .into_iter()
            .filter(|file| {
                std::fs::read_to_string(root().join(file))
                    .is_ok_and(|t| !code_references(&t, "Policy::SHIPPED").is_empty())
            })
            .collect();
        assert!(
            reached.len() >= 2,
            "expected the decider and at least one reporter to use              Policy::SHIPPED, found {reached:?}"
        );
    }

    #[test]
    fn the_reference_extractor_reads_code_and_not_prose() {
        // The two checks above are only worth having if this is right. Too
        // greedy and every mention in a doc comment is a failure; too narrow and
        // both pass by finding nothing.
        let source = "/// A doc comment naming Policy::CLOSED is documentation.
// So is a line comment naming Policy::CLOSED.
let a = Policy::CLOSED;
let b = Policy::SHIPPED; // Policy::CLOSED here is a trailing comment
";
        assert_eq!(
            code_references(source, "Policy::CLOSED"),
            vec![3],
            "only the real binding counts"
        );
        assert_eq!(code_references(source, "Policy::SHIPPED"), vec![4]);
    }

    #[test]
    fn the_extractor_stops_at_the_test_module() {
        // A test may name anything. Without this, every crate with a test module
        // referencing a constant would be a permanent false positive, and a
        // check that cries wolf is one somebody deletes.
        let source = "let real = Policy::SHIPPED;
#[cfg(test)]
mod tests {
    let fixture = Policy::CLOSED;
}
";
        assert!(code_references(source, "Policy::CLOSED").is_empty());
        assert_eq!(code_references(source, "Policy::SHIPPED"), vec![1]);
    }

    #[test]
    fn crate_sources_finds_source_and_excludes_what_it_is_told_to() {
        let all = crate_sources("crates/radar-risk/");
        assert!(!all.is_empty(), "no crate sources found at all");
        assert!(
            all.iter().all(|f| !f.starts_with("crates/radar-risk/")),
            "the exclusion did not apply"
        );
        assert!(
            all.iter().any(|f| f.starts_with("crates/radar-serve/src/")),
            "expected radar-serve's sources: {all:?}"
        );
        // `tests/` directories are out by construction, not by exclusion.
        assert!(
            all.iter().all(|f| f.contains("/src/")),
            "a non-src file was included"
        );
    }
}
