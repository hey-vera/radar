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
    fn no_markdown_document_is_invisible_to_these_checks() {
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
