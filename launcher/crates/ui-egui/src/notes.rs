//! Release notes: GitHub's generated Markdown, rendered inline without a Markdown engine.
//!
//! The line classifier understands the shape of GitHub's generated notes (`## What's Changed`,
//! `* Change ... by @user in https://…`); the spans parser renders the inline Markdown inside
//! each line: `**strong**`, `*em*` / `_em_`, ` `code`` and [links](url). URLs the launcher
//! refuses to open stay literal text.

use egui::{RichText, Ui};

use crate::theme::Tokens;

/// One inline run of text, after parsing the Markdown that shows up in GitHub's release notes:
/// `**strong**`, `*em*`/`_em_`, ` `code`` and `[links](url)`.
#[derive(Debug, PartialEq, Eq)]
enum Span<'a> {
    Text(&'a str),
    Strong(&'a str),
    Em(&'a str),
    Code(&'a str),
    Link(&'a str, &'a str),
}

/// Splits `text` into inline spans. Markers that never close are literal text, as CommonMark
/// does; `*emphasis*` only pairs with its own kind. A link with an unsafe URL stays literal too.
fn spans(text: &str) -> Vec<Span<'_>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < text.len() {
        let s = &text[i..];
        let mut advance: Option<(Span<'_>, usize)> = None;
        if let Some(inner) = s.strip_prefix("**") {
            if let Some(end) = inner.find("**") {
                advance = Some((Span::Strong(&inner[..end]), 2 + end + 2));
            }
        } else if let Some(inner) = s.strip_prefix("__") && let Some(end) = inner.find("__") {
            advance = Some((Span::Strong(&inner[..end]), 2 + end + 2));
        } else if let Some(inner) = s.strip_prefix('`') {
            if let Some(end) = inner.find('`') {
                advance = Some((Span::Code(&inner[..end]), 1 + end + 1));
            }
        } else if let Some(inner) = s.strip_prefix('[') && let Some(close) = inner.find("](") {
            let after = &inner[close + 2..];
            if let Some(paren) = after.find(')') {
                let url = &after[..paren];
                if artcraft_launcher_core::release::safe_web_link(url) {
                    advance = Some((Span::Link(&inner[..close], url), 1 + close + 2 + paren + 1));
                }
            }
        } else if let Some(inner) = s.strip_prefix('*') {
            if let Some(end) = inner.find('*') {
                advance = Some((Span::Em(&inner[..end]), 1 + end + 1));
            }
        } else if let Some(inner) = s.strip_prefix('_') {
            if let Some(end) = inner.find('_') {
                advance = Some((Span::Em(&inner[..end]), 1 + end + 1));
            }
        }
        match advance {
            Some((span, len)) => {
                if start < i {
                    out.push(Span::Text(&text[start..i]));
                }
                out.push(span);
                i += len;
                start = i;
            }
            None => {
                i += s.chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    if start < text.len() {
        out.push(Span::Text(&text[start..]));
    }
    out
}

/// A common look for lines whose spans are read the same, whatever their size and face.
#[derive(Clone)]
struct SpanStyle {
    family: egui::FontFamily,
    size: f32,
    color: egui::Color32,
}

impl SpanStyle {
    fn text(&self, s: &str) -> egui::RichText {
        egui::RichText::new(s).family(self.family.clone()).size(self.size).color(self.color)
    }

    fn mono(&self, s: &str, t: &Tokens) -> egui::RichText {
        self.text(s).family(crate::theme::mono()).size((self.size - 1.0).max(1.0)).background_color(t.sunken)
    }
}

/// Draws the spans inline, one wrapping row: formatting survives, widgets and all.
fn spans_ui(ui: &mut Ui, text: &str, style: SpanStyle, t: &Tokens) {
    for span in spans(text) {
        match span {
            Span::Text(s) => {
                ui.label(style.text(s));
            }
            Span::Strong(s) => {
                ui.label(style.text(s).strong());
            }
            Span::Em(s) => {
                ui.label(style.text(s).italics());
            }
            Span::Code(s) => {
                ui.label(style.mono(s, t));
            }
            Span::Link(s, url) => {
                ui.hyperlink_to(style.text(s).underline(), url);
            }
        }
    }
}

/// One line of notes, classified.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Line<'a> {
    Heading(&'a str),
    /// A bullet: its text, and the pull request link GitHub appends (`… in https://…/pull/12`).
    Bullet(&'a str, Option<&'a str>),
    Text(&'a str),
    Blank,
}

pub(crate) fn classify(line: &str) -> Line<'_> {
    let l = line.trim();
    if l.is_empty() {
        return Line::Blank;
    }
    if let Some(h) = l.strip_prefix('#') {
        return Line::Heading(h.trim_start_matches('#').trim());
    }
    if let Some(b) = l.strip_prefix("* ").or_else(|| l.strip_prefix("- ")) {
        if let Some((text, url)) = b.rsplit_once(" in https://")
            && !url.contains(' ')
        {
            let start = b.len() - url.len() - "https://".len();
            return Line::Bullet(text, b.get(start..));
        }
        return Line::Bullet(b, None);
    }
    Line::Text(l)
}

/// How many changes show before the rest folds away.
const SHOWN: usize = 6;

pub(crate) fn show(ui: &mut Ui, notes: &str) {
    show_scaled(ui, notes, 1.0);
}

/// The notes at `scale` times their usual size (the notes dialog's own zoom).
pub(crate) fn show_scaled(ui: &mut Ui, notes: &str, scale: f32) {
    ui.scope(|ui| {
        for font in ui.style_mut().text_styles.values_mut() {
            font.size *= scale;
        }
        show_lines(ui, notes, scale);
    });
}

/// The notes, short: the first few changes in full, the rest behind "Show N more changes", and the
/// link to the full changelog always in view.
fn show_lines(ui: &mut Ui, notes: &str, scale: f32) {
    let t = Tokens::get(ui.ctx());
    if notes.trim().is_empty() {
        ui.label(RichText::new(tl!("No notes were published with this release.")).color(t.muted));
        return;
    }
    let (changelog, lines): (Vec<Line<'_>>, Vec<Line<'_>>) = notes.lines().map(classify).partition(|l| full_changelog(l).is_some());
    let bullets = lines.iter().filter(|l| matches!(l, Line::Bullet(..))).count();
    let (head, rest) = lines.split_at(fold_at(&lines).unwrap_or(lines.len()));
    for line in head {
        draw(ui, line, &t, scale);
    }
    if !rest.is_empty() {
        let label = crate::i18n::fmt(tl!("Show {n} more changes"), &[("n", &bullets.saturating_sub(SHOWN).to_string())]);
        egui::CollapsingHeader::new(label).id_salt(ui.scope_id().with(("more-notes", notes.len()))).show(ui, |ui| {
            for line in rest {
                draw(ui, line, &t, scale);
            }
        });
    }
    for line in &changelog {
        if let Some(url) = full_changelog(line) {
            ui.add_space(6.0);
            ui.hyperlink_to(tl!("Full changelog"), url);
        }
    }
}

/// Where a long list folds: just before its (SHOWN + 1)th change. A short list is shown whole.
fn fold_at(lines: &[Line<'_>]) -> Option<usize> {
    let bullets = lines.iter().filter(|l| matches!(l, Line::Bullet(..))).count();
    if bullets <= SHOWN + 1 {
        return None;
    }
    lines.iter().enumerate().filter(|(_, l)| matches!(l, Line::Bullet(..))).nth(SHOWN).map(|(i, _)| i)
}

/// The address on a "**Full Changelog**: …" line, when it is a safe web link.
fn full_changelog<'a>(line: &Line<'a>) -> Option<&'a str> {
    let Line::Text(text) = line else { return None };
    text.strip_prefix("**Full Changelog**: ").map(str::trim).filter(|url| artcraft_launcher_core::release::safe_web_link(url))
}

/// The usual size of body text as it exists inside this UI scope (the notes dialog zooms it).
fn body_size(ui: &Ui) -> f32 {
    ui.style().text_styles.get(&egui::TextStyle::Body).map_or(14.0, |f| f.size)
}

/// `true` when `text` holds nothing the spans parser would turn into markup.
fn no_markup(text: &str) -> bool {
    spans(text).into_iter().all(|span| matches!(span, Span::Text(_)))
}

fn draw(ui: &mut Ui, line: &Line<'_>, t: &Tokens, scale: f32) {
    match line {
        Line::Heading(h) => {
            ui.add_space(6.0);
            if no_markup(h) {
                ui.label(crate::widgets::heading(h, 16.0 * scale, t.ink_strong));
            } else {
                let size = 16.0 * scale;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    spans_ui(ui, h, SpanStyle { family: crate::theme::display(), size, color: t.ink_strong }, t);
                });
            }
        }
        Line::Bullet(text, link) => {
            // The author's handle is noise in a list of what changed.
            let text = text.rsplit_once(" by @").map_or(*text, |(change, _)| change);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new("•").color(t.faint));
                if no_markup(text) {
                    ui.label(RichText::new(text).color(t.ink));
                } else {
                    let size = body_size(ui);
                    spans_ui(ui, text, SpanStyle { family: egui::FontFamily::Proportional, size, color: t.ink }, t);
                }
                if let Some(url) = link.filter(|u| artcraft_launcher_core::release::safe_web_link(u)) {
                    let short = url.rsplit('/').next().map_or_else(|| tl!("link").to_string(), |n| format!("#{n}"));
                    ui.hyperlink_to(RichText::new(short).size(12.0 * scale), url);
                }
            });
        }
        Line::Text(text) => {
            if no_markup(text) {
                ui.label(RichText::new(*text).color(t.muted));
            } else {
                let size = body_size(ui);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    spans_ui(ui, text, SpanStyle { family: egui::FontFamily::Proportional, size, color: t.muted }, t);
                });
            }
        }
        Line::Blank => {
            ui.add_space(2.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_githubs_generated_notes() {
        assert_eq!(classify("## What's Changed"), Line::Heading("What's Changed"));
        assert_eq!(
            classify("* Add the Patch tool (#213) by @a in https://github.com/storytold/photocraft/pull/381"),
            Line::Bullet("Add the Patch tool (#213) by @a", Some("https://github.com/storytold/photocraft/pull/381"))
        );
        assert_eq!(classify("- plain bullet"), Line::Bullet("plain bullet", None));
        assert_eq!(classify("   "), Line::Blank);
        assert_eq!(classify("hello"), Line::Text("hello"));
    }

    #[test]
    fn spans_parse_inline_markdown() {
        assert_eq!(spans("plain"), vec![Span::Text("plain")]);
        assert_eq!(spans("**bold**"), vec![Span::Strong("bold")]);
        assert_eq!(spans("__bold__"), vec![Span::Strong("bold")]);
        assert_eq!(spans("*italic*"), vec![Span::Em("italic")]);
        assert_eq!(spans("_italic_"), vec![Span::Em("italic")]);
        assert_eq!(spans("`code`"), vec![Span::Code("code")]);
        assert_eq!(spans("[a link](https://example.com)"), vec![Span::Link("a link", "https://example.com")]);
        assert_eq!(
            spans("before **b** and `c` after"),
            vec![Span::Text("before "), Span::Strong("b"), Span::Text(" and "), Span::Code("c"), Span::Text(" after")]
        );
        // Unclosed markers are literal text, as CommonMark does.
        assert_eq!(spans("**never closes"), vec![Span::Text("**never closes")]);
        assert_eq!(spans("a * b"), vec![Span::Text("a * b")]);
        // A URL the launcher refuses to open stays literal.
        assert_eq!(spans("[x](javascript:alert(1))"), vec![Span::Text("[x](javascript:alert(1))")]);
        // A link ends at its first `)`.
        assert_eq!(spans("[x](https://example.com)"), vec![Span::Link("x", "https://example.com")]);
    }

    #[test]
    fn a_long_list_folds_after_six_changes_and_a_short_one_does_not() {
        let many: Vec<Line> = std::iter::once(Line::Heading("What's Changed")).chain((0..10).map(|_| Line::Bullet("change", None))).collect();
        // The heading, then six bullets: the seventh bullet (index 7) is the first one folded.
        assert_eq!(fold_at(&many), Some(7));
        let few: Vec<Line> = (0..7).map(|_| Line::Bullet("change", None)).collect();
        assert_eq!(fold_at(&few), None);
        assert_eq!(fold_at(&[]), None);
    }
}
