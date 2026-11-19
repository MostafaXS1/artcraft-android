#!/usr/bin/env bash
# Performance optimizations for WordCraft on Android (large document typing lag).
#
# 1. Fast typing in Session::run:
#    Avoid cloning the entire 100-step undo history and redo stacks on any typing command
#    (letters, punctuation, numbers, spaces, enter, backspace, delete), and skip the
#    full-document block-by-block traversal in unchanged(self).
# 2. UI layer debounce for full-document status counts and missing fonts scanning in chrome.rs:
#    During active continuous typing (and within 500ms of keystrokes), reuse cached word/char
#    counts and missing font memos instead of synchronously extracting plain text, running
#    Unicode word segmentation across all paragraphs, and walking all pages every frame.
# 3. Zero-allocation page fingerprinting for fills and rules in page_content_key.
# 4. Pre-allocate line indexing capacity in pagination pass (index_lines).
# 5. Fast incremental layout cache state and zero-allocation env_hash.
# 6. Fast single-paragraph incremental layout for all in-line typing and line editing:
#    When typing inside an ordinary paragraph without changing line count or height,
#    re-lay only that single paragraph and reuse surrounding document page geometry.
#
# Only WordCraft is touched.
set -euo pipefail

SRC="${1:?source dir}"
APP="${2:?app}"
TITLE="${3:?title}"

if [[ "$APP" != "wordcraft" ]]; then
    exit 0
fi

python3 - "$SRC" "$APP" <<'PY'
from pathlib import Path
import sys

src, app = Path(sys.argv[1]), Path(sys.argv[2])
fixed = []

def fix_source(rel: str, old: str, new: str) -> None:
    path = src / rel
    try:
        text = path.read_text()
    except OSError:
        return
    if old not in text or new in text or text.count(old) != 1:
        return
    path.write_text(text.replace(old, new, 1))
    fixed.append(rel)

# 1. Fast typing in Session::run: avoid history/redo cloning and skip full-doc comparison
fix_source(
    "crates/engine/src/session/run.rs",
    """        let typing = spec.id == "text.insert" || spec.id == "equation.type";
        if spec.mutates && !typing {
            self.typing_open = false;
        }""",
    """        let typing = spec.id == "text.insert"
            || spec.id == "equation.type"
            || spec.id == "text.backspace"
            || spec.id == "text.delete"
            || spec.id == "text.newParagraph"
            || spec.id == "equation.backspace";
        if spec.mutates && !typing {
            self.typing_open = false;
        }""",
)

# 1b. Gated proofing in session layout: debounce spelling/grammar during active typing session
fix_source(
    "crates/engine/src/session/layout.rs",
    """        let rev = (self.rev, wordcraft_proof::dictionary_generation());
        let (proofing, numeral) = (self.view.proofing, self.view.numeral);""",
    """        let rev = (self.rev, wordcraft_proof::dictionary_generation());
        let (proofing, numeral) = (self.view.proofing && !self.typing_open, self.view.numeral);""",
)

fix_source(
    "crates/engine/src/session/layout.rs",
    """            hide_marked: deleted.1,
            deletions_in_balloons: deleted.2,
            proofing: self.view.proofing,
            numeral: self.view.numeral.resolve(SYSTEM_LOCALE.get().map(String::as_str)),""",
    """            hide_marked: deleted.1,
            deletions_in_balloons: deleted.2,
            proofing,
            numeral: self.view.numeral.resolve(SYSTEM_LOCALE.get().map(String::as_str)),""",
)

fix_source(
    "crates/engine/src/session/layout.rs",
    """        let l = Arc::new(wordcraft_layout::layout(&self.doc, &mut self.cache, &opts));
        let [newest, _] = std::mem::take(&mut self.layout);
        self.layout =
            [Some((rev, width, height, mode, spacing, l.clone(), self.view.proofing, outline, deleted, self.view.numeral, typing, shown)), newest];""",
    """        let l = Arc::new(wordcraft_layout::layout(&self.doc, &mut self.cache, &opts));
        let [newest, _] = std::mem::take(&mut self.layout);
        self.layout =
            [Some((rev, width, height, mode, spacing, l.clone(), proofing, outline, deleted, self.view.numeral, typing, shown)), newest];""",
)

fix_source(
    "crates/engine/src/session/run.rs",
    """        // The undo stacks are snapshotted whole, not as a length: commands run nested commands,
        // whose checkpoints, undos and redos (and evictions at the limit) must be put back too.
        let ime_epoch_before = spec.mutates.then_some(self.ime_body_epoch);
        // Commands may leave an equation (a whole-body replacement) even after they fail; put
        // the editing session back with the document.
        let math_before = spec.mutates.then(|| self.math.clone());
        let before_doc: Option<RunSnapshot> = if spec.mutates {
            Some((
                self.doc.clone(),
                self.sel.clone(),
                self.history.clone(),
                self.redo.clone(),
                self.typing_open,
                self.undo_evicted,
                self.column.clone(),
            ))
        } else {
            None
        };""",
    """        // The undo stacks are snapshotted whole, not as a length: commands run nested commands,
        // whose checkpoints, undos and redos (and evictions at the limit) must be put back too.
        let ime_epoch_before = spec.mutates.then_some(self.ime_body_epoch);
        // Commands may leave an equation (a whole-body replacement) even after they fail; put
        // the editing session back with the document.
        let math_before = spec.mutates.then(|| self.math.clone());
        let fast_typing = !self.history.is_empty() && matches!(
            id,
            "text.insert"
                | "equation.type"
                | "equation.backspace"
                | "text.backspace"
                | "text.delete"
                | "text.newParagraph"
                | "text.lineBreak"
        );
        let (history_snap, redo_snap) = if fast_typing {
            (Vec::new(), Vec::new())
        } else {
            (self.history.clone(), self.redo.clone())
        };
        let before_doc: Option<RunSnapshot> = if spec.mutates {
            Some((
                self.doc.clone(),
                self.sel.clone(),
                history_snap,
                redo_snap,
                self.typing_open,
                self.undo_evicted,
                self.column.clone(),
            ))
        } else {
            None
        };""",
)

fix_source(
    "crates/engine/src/session/run.rs",
    """                if let Some((d, s, h, r, t, ev, c)) = before_doc {
                    self.doc = d;
                    self.sel = s;
                    self.history = h;
                    self.redo = r;
                    self.typing_open = t;
                    // The steps the command pushed out are back, so they no longer count as gone.
                    self.undo_evicted = ev;
                    self.column = c;
                    // Its step is gone with them: its undo group has none to merge into.
                    if pushed {
                        self.join_anchor = false;
                    }
                }""",
    """                if let Some((d, s, h, r, t, ev, c)) = before_doc {
                    self.doc = d;
                    self.sel = s;
                    if !h.is_empty() {
                        self.history = h;
                    } else if pushed {
                        self.history.pop();
                    }
                    if !r.is_empty() {
                        self.redo = r;
                    }
                    self.typing_open = t;
                    // The steps the command pushed out are back, so they no longer count as gone.
                    self.undo_evicted = ev;
                    self.column = c;
                    // Its step is gone with them: its undo group has none to merge into.
                    if pushed {
                        self.join_anchor = false;
                    }
                }""",
)

fix_source(
    "crates/engine/src/session/run.rs",
    """            // waiting at the caret as they were. A command that replaces the document —
            // a mail merge, a comparison — is not an edit of this one, whatever the new
            // document holds: `replaces_document` names the commands that may replace
            // it, and `document_id` catches any other that does (through
            // `set_document`, which starts a new history the old one must not overwrite).
            let unchanged = |s: &Session| match (before_doc, pending_before) {
                (Some((doc, sel, _, _, _, _, column)), Some(pending)) => {
                    !replaces_document && s.document_id == document_id_before && s.unchanged(doc, sel, column, pending)
                }
                _ => false,
            };
            let changed = !unchanged(self);
            if changed {
                // The layout the connectors are rerouted against is the new one.
                self.touch();
            }
            // Glued connectors follow the shapes they join.
            let rerouted = crate::cmd::connectors::reroutes_after(id) && crate::cmd::connectors::reroute(self);
            if rerouted {
                self.touch();
            }
            // A command that changed nothing is not an edit (#23): it adds no undo
            // step, marks no unsaved changes, forces no relayout and leaves the redo
            // stack alone. What it left behind is compared with what it started from,
            // after the reroute: a reroute that moved a connector is an edit of its own.
            no_op = !changed && !rerouted && unchanged(self);
            if no_op {
                // The step the dispatch pushed is not an edit: put the stacks back as
                // they were, so the redo stack survives and no step is evicted. The
                // group it belonged to is left without a step of its own, so the next
                // command in it starts one.
                if let Some((_, _, history, redo, typing_open, evicted, _)) = before_doc {
                    self.history = history.clone();
                    self.redo = redo.clone();
                    self.typing_open = *typing_open;
                    self.undo_evicted = *evicted;
                }""",
    """            // waiting at the caret as they were. A command that replaces the document —
            // a mail merge, a comparison — is not an edit of this one, whatever the new
            // document holds: `replaces_document` names the commands that may replace
            // it, and `document_id` catches any other that does (through
            // `set_document`, which starts a new history the old one must not overwrite).
            let is_always_mutating = matches!(
                id,
                "text.insert"
                    | "equation.type"
                    | "equation.backspace"
                    | "text.backspace"
                    | "text.delete"
                    | "text.newParagraph"
                    | "text.lineBreak"
            );
            let unchanged = |s: &Session| {
                if is_always_mutating {
                    return false;
                }
                match (before_doc, pending_before) {
                    (Some((doc, sel, _, _, _, _, column)), Some(pending)) => {
                        !replaces_document && s.document_id == document_id_before && s.unchanged(doc, sel, column, pending)
                    }
                    _ => false,
                }
            };
            let changed = !unchanged(self);
            if changed {
                // The layout the connectors are rerouted against is the new one.
                self.touch();
            }
            // Glued connectors follow the shapes they join.
            let rerouted = crate::cmd::connectors::reroutes_after(id) && crate::cmd::connectors::reroute(self);
            if rerouted {
                self.touch();
            }
            // A command that changed nothing is not an edit (#23): it adds no undo
            // step, marks no unsaved changes, forces no relayout and leaves the redo
            // stack alone. What it left behind is compared with what it started from,
            // after the reroute: a reroute that moved a connector is an edit of its own.
            no_op = !changed && !rerouted && unchanged(self);
            if no_op {
                // The step the dispatch pushed is not an edit: put the stacks back as
                // they were, so the redo stack survives and no step is evicted. The
                // group it belonged to is left without a step of its own, so the next
                // command in it starts one.
                if let Some((_, _, history, redo, typing_open, evicted, _)) = before_doc {
                    if !history.is_empty() {
                        self.history = history.clone();
                    } else if pushed {
                        self.history.pop();
                    }
                    if !redo.is_empty() {
                        self.redo = redo.clone();
                    }
                    self.typing_open = *typing_open;
                    self.undo_evicted = *evicted;
                }""",
)

# 2. UI layer debounce for full-document status counts and missing fonts scanning in chrome.rs
fix_source(
    "crates/ui-egui/src/chrome.rs",
    """    pub fn cached_missing_fonts(&mut self, layout: &std::sync::Arc<wordcraft_layout::DocLayout>) -> MissingFontsCache {
        if !self.missing_fonts.0.upgrade().is_some_and(|seen| std::sync::Arc::ptr_eq(&seen, layout)) {
            let found = wordcraft_engine::cmd::font::missing_fonts(&self.session.doc, layout);
            let warnings = found
                .iter()
                .filter(|f| !f.metric_compatible)
                .map(|f| (wordcraft_engine::cmd::font::font_key(&f.font), f.shown_as.join(", ")))
                .collect();
            self.missing_fonts = (std::sync::Arc::downgrade(layout), (std::sync::Arc::new(found), std::sync::Arc::new(warnings)));
        }
        self.missing_fonts.1.clone()
    }""",
    """    pub fn cached_missing_fonts(&mut self, layout: &std::sync::Arc<wordcraft_layout::DocLayout>) -> MissingFontsCache {
        if !self.missing_fonts.0.upgrade().is_some_and(|seen| std::sync::Arc::ptr_eq(&seen, layout)) {
            if (self.session.typing_open() || (crate::now_ms() - self.canvas.caret_visible_since < 500.0))
                && self.missing_fonts.0.upgrade().is_some()
            {
                return self.missing_fonts.1.clone();
            }
            let found = wordcraft_engine::cmd::font::missing_fonts(&self.session.doc, layout);
            let warnings = found
                .iter()
                .filter(|f| !f.metric_compatible)
                .map(|f| (wordcraft_engine::cmd::font::font_key(&f.font), f.shown_as.join(", ")))
                .collect();
            self.missing_fonts = (std::sync::Arc::downgrade(layout), (std::sync::Arc::new(found), std::sync::Arc::new(warnings)));
        }
        self.missing_fonts.1.clone()
    }""",
)

fix_source(
    "crates/ui-egui/src/chrome.rs",
    """    pub fn cached_status_counts(&mut self) -> StatusCounts {
        // The count option changes the counts without a document change.
        let key = self.session.rev().wrapping_mul(2) | u64::from(self.session.prefs.count_notes);
        if self.status_counts.0 != key {
            let text = self.session.counted_text();
            self.status_counts = (
                key,
                StatusCounts {
                    words: self.session.word_count(),
                    chars: wordcraft_doc::count_chars(&text),
                    chars_no_spaces: wordcraft_doc::count_chars_no_spaces(&text),
                },
            );
        }
        self.status_counts.1
    }""",
    """    pub fn cached_status_counts(&mut self) -> StatusCounts {
        // The count option changes the counts without a document change.
        let key = self.session.rev().wrapping_mul(2) | u64::from(self.session.prefs.count_notes);
        if self.status_counts.0 != key {
            if (self.session.typing_open() || (crate::now_ms() - self.canvas.caret_visible_since < 500.0))
                && self.status_counts.0 != crate::window_ui::NOT_COUNTED.0
            {
                return self.status_counts.1;
            }
            let text = self.session.counted_text();
            self.status_counts = (
                key,
                StatusCounts {
                    words: self.session.word_count(),
                    chars: wordcraft_doc::count_chars(&text),
                    chars_no_spaces: wordcraft_doc::count_chars_no_spaces(&text),
                },
            );
        }
        self.status_counts.1
    }""",
)

# 3. Zero-allocation page fingerprinting for fills and rules in page_content_key and view_key
fix_source(
    "crates/ui-egui/src/canvas/pages.rs",
    """    format!("{:?}{:?}{:?}", app.session.doc.settings.page_color, app.session.doc.settings.page_picture, app.session.doc.settings.watermark)
        .hash(&mut h);""",
    """    (
        app.session.doc.settings.page_color.map(|c| (c.0, c.1, c.2)),
        app.session.doc.settings.page_picture.as_ref().map(|p| (p.media.as_str(), p.fill_type.as_str(), p.title.as_str())),
        app.session.doc.settings.watermark.as_ref().map(|w| (w.text.as_str(), w.font.as_str(), w.color.0, w.color.1, w.color.2, w.diagonal, w.semitransparent, w.size.map(|s| (s[0].to_bits(), s[1].to_bits())))),
    )
        .hash(&mut h);""",
)

fix_source(
    "crates/ui-egui/src/canvas/pages.rs",
    """                Placed::Fill { rect, color } => format!("{rect:?}{color:?}").hash(&mut h),
                Placed::Rule { x0, y0, x1, y1, border } => format!("{x0}{y0}{x1}{y1}{border:?}").hash(&mut h),""",
    """                Placed::Fill { rect, color } => {
                    (rect.x.to_bits(), rect.y.to_bits(), rect.w.to_bits(), rect.h.to_bits(), color.0, color.1, color.2).hash(&mut h);
                }
                Placed::Rule { x0, y0, x1, y1, border } => {
                    (x0.to_bits(), y0.to_bits(), x1.to_bits(), y1.to_bits(), border.style as usize, border.width.to_bits(), border.color.map(|c| (c.0, c.1, c.2)), border.space.to_bits()).hash(&mut h);
                }""",
)

# 4. Pre-allocate line indexing capacity in pagination pass (index_lines)
fix_source(
    "crates/layout/src/pass.rs",
    """fn index_lines(pages: &[Page]) -> HashMap<(StoryRef, Path), Vec<(usize, usize)>> {
    let mut index: HashMap<(StoryRef, Path), Vec<(usize, usize)>> = HashMap::new();""",
    """fn index_lines(pages: &[Page]) -> HashMap<(StoryRef, Path), Vec<(usize, usize)>> {
    let mut index: HashMap<(StoryRef, Path), Vec<(usize, usize)>> = HashMap::with_capacity(pages.iter().map(|p| p.items.len()).sum());""",
)

# 5. Fast incremental layout cache state and zero-allocation env_hash
fix_source(
    "crates/layout/src/cache.rs",
    """use super::{
    Arc, Blocks, Document, Hash, HashMap, HashSet, Hasher, HideDeleted, InlineObject, LayoutOptions, Page, ParaLayout, Paragraph, SectionProps,
    StoryRef, ViewMode, fields, styleref,
};""",
    """use super::{
    Arc, Blocks, DocLayout, Document, Hash, HashMap, HashSet, Hasher, HideDeleted, InlineObject, LayoutOptions, Page, ParaLayout, Paragraph, SectionProps,
    StoryRef, ViewMode, fields, styleref,
};""",
)

fix_source(
    "crates/layout/src/cache.rs",
    """    pub hits: u64,
    pub misses: u64,
}""",
    """    pub hits: u64,
    pub misses: u64,
    pub(crate) last_body: Option<Blocks>,
    pub(crate) last_layout: Option<DocLayout>,
}""",
)

fix_source(
    "crates/layout/src/cache.rs",
    """pub(crate) fn env_hash(doc: &Document, opts: &LayoutOptions) -> u64 {
    let s = serde_json::to_string(&(&doc.styles, &doc.numbering, doc.settings.default_tab, &doc.settings.footnote_format)).unwrap_or_default();
    // Outline view lays paragraphs out changed (no indents; plain without text formatting).
    let outline = (opts.view == ViewMode::Outline).then(|| opts.outline.as_ref().is_some_and(|o| o.plain));
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);""",
    """pub(crate) fn env_hash(doc: &Document, opts: &LayoutOptions) -> u64 {
    // Outline view lays paragraphs out changed (no indents; plain without text formatting).
    let outline = (opts.view == ViewMode::Outline).then(|| opts.outline.as_ref().is_some_and(|o| o.plain));
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        Arc::as_ptr(&doc.styles) as usize,
        Arc::as_ptr(&doc.numbering) as usize,
        doc.settings.default_tab.to_bits(),
        doc.settings.footnote_format,
    )
        .hash(&mut h);""",
)

fix_source(
    "crates/layout/src/pass.rs",
    """    if env != cache.env {
        cache.paras.clear();
        cache.measures.clear();
        cache.env = env;
    }""",
    """    if env != cache.env {
        cache.paras.clear();
        cache.measures.clear();
        cache.env = env;
        cache.last_body = None;
        cache.last_layout = None;
    }""",
)

# 6. Fast single-paragraph incremental layout for all in-line typing and line editing:
# When typing inside an ordinary paragraph without altering line count or height,
# re-lay only that single paragraph and reuse the surrounding document page geometry.
fix_source(
    "crates/layout/src/lib.rs",
    """/// Lay out the whole document.
pub fn layout(doc: &Document, cache: &mut LayoutCache, opts: &LayoutOptions) -> DocLayout {
    #[cfg(test)]
    crate::test_fonts();
    let t0 = now_ms();""",
    """/// Fast-path incremental layout during continuous typing and line editing: when typing inside
/// an ordinary paragraph without altering its line count or height, re-lay only that single
/// paragraph and reuse the surrounding document page geometry.
fn try_fast_typing_layout(
    doc: &Document,
    cache: &mut LayoutCache,
    opts: &LayoutOptions,
    t0: f64,
) -> Option<DocLayout> {
    if opts.view != ViewMode::Print {
        return None;
    }
    let last_layout = cache.last_layout.as_ref()?;
    let last_body = cache.last_body.as_ref()?;
    if last_layout.pages.is_empty() {
        return None;
    }

    let last_page_idx = last_layout.pages.len() - 1;
    let last_page = last_layout.pages.get(last_page_idx)?;
    let sections = doc.sections();
    let (_, sect) = sections.get(last_page.section)?;
    let cols = sect.column_boxes();
    if cols.len() != 1 {
        return None;
    }
    let col_w = last_page.body.w;

    let item_bottom = |item: &Placed| match item {
        Placed::Lines { y, para, .. } => *y + para.height,
        Placed::Fill { rect, .. }
        | Placed::Image { rect, .. }
        | Placed::Shape { rect, .. }
        | Placed::Graphic { rect, .. }
        | Placed::Clip { rect, .. }
        | Placed::Cell { rect, .. }
        | Placed::Object { rect, .. } => rect.y + rect.h,
        Placed::Rule { y1, .. } => *y1,
    };

    // Case 1: Fast-path for Enter / paragraph split (doc.body.len() == last_body.len() + 1)
    if doc.body.len() == last_body.len() + 1 {
        let mut split_idx = None;
        for i in 0..last_body.len() {
            if !Arc::ptr_eq(&last_body[i], &doc.body[i]) {
                split_idx = Some(i);
                break;
            }
        }
        let split_idx = split_idx.unwrap_or(last_body.len() - 1);
        for k in (split_idx + 1)..last_body.len() {
            if !Arc::ptr_eq(&last_body[k], &doc.body[k + 1]) {
                return None;
            }
        }

        let p0 = doc.body.get(split_idx)?.as_para()?;
        let p1 = doc.body.get(split_idx + 1)?.as_para()?;
        if !p0.objects.is_empty() || p0.section.is_some() || !p1.objects.is_empty() || p1.section.is_some() {
            return None;
        }
        if p0.props.page_break_before.unwrap_or(false) || p1.props.page_break_before.unwrap_or(false) {
            return None;
        }

        let path0 = Path::top(split_idx);
        let pieces0 = last_layout.index.get(&(StoryRef::Body, path0.clone()))?;
        if pieces0.len() != 1 {
            return None;
        }
        let (page_idx, item_idx) = pieces0[0];
        if page_idx + 1 != last_layout.pages.len() {
            return None;
        }
        let page = last_layout.pages.get(page_idx)?;
        let item0 = page.items.get(item_idx)?;
        let Placed::Lines {
            story,
            path: ref item_path0,
            para: ref prev_pl0,
            l0,
            l1,
            x: item_x,
            y: item_y,
            turn,
        } = *item0 else {
            return None;
        };
        if story != StoryRef::Body || *item_path0 != path0 || l0 != 0 || l1 != prev_pl0.lines.len() {
            return None;
        }
        if turn != TextDirection::Horizontal {
            return None;
        }
        if prev_pl0.has_page_fields
            || !prev_pl0.notes.is_empty()
            || prev_pl0.drop_cap.is_some()
            || prev_pl0.rp.page_break_before
            || prev_pl0.rp.borders.is_some()
            || prev_pl0.rp.shading.is_some()
            || prev_pl0.rp.keep_next
            || prev_pl0.label.is_some()
            || p0.props.numbering.is_some()
            || p1.props.numbering.is_some()
        {
            return None;
        }

        for j in (item_idx + 1)..page.items.len() {
            match &page.items[j] {
                Placed::Lines { story, .. } if *story == StoryRef::Body => {}
                _ => return None,
            }
        }

        let fields = fields::FieldCtx::default();
        let env = para::ParaEnv {
            doc,
            width: col_w,
            label: None,
            fields: &fields,
            show_hidden: opts.show_hidden,
            field_codes: opts.field_codes,
            hide_deleted: opts.hides_deleted(StoryRef::Body),
            table: None,
            proofing: opts.proofing.then_some(opts.proof),
            typing_word: None,
            numeral: opts.numeral,
            exclusions: &[],
            eq_number: 0,
            read_line_spacing: opts.read_line_spacing,
            grid: para::Grid::of(doc, sect),
            fit_objects: false,
            fitted: &[],
        };
        let new_pl0 = para::layout_para(p0, &env);
        let new_pl1 = para::layout_para(p1, &env);

        let space_between = p1.props.space_before.unwrap_or(0.0);
        let total_new_height = new_pl0.height + space_between + new_pl1.height;
        let delta = total_new_height - prev_pl0.height;

        let max_bottom = page.items.iter().map(item_bottom).fold(0.0_f32, f32::max);
        if max_bottom + delta > page.body.bottom() + 0.1 {
            return None;
        }

        let arc_pl0 = Arc::new(new_pl0);
        let arc_pl1 = Arc::new(new_pl1);
        let path1 = Path::top(split_idx + 1);

        let key0 = cache::Key {
            rev: p0.rev,
            width: col_w.to_bits(),
            label: None,
            page: None,
            page_from: 0,
            table: 0,
            notes: 0,
            excl: 0,
            eq: 0,
            read_spacing: None,
            grid: env.grid.key(),
            fit: false,
            fitted: 0,
            hide_deleted: opts.hides_deleted(StoryRef::Body),
            sref: 0,
            typing: None,
        };
        cache.used.insert(key0.clone());
        cache.paras.insert(key0, arc_pl0.clone());

        let key1 = cache::Key {
            rev: p1.rev,
            width: col_w.to_bits(),
            label: None,
            page: None,
            page_from: 0,
            table: 0,
            notes: 0,
            excl: 0,
            eq: 0,
            read_spacing: None,
            grid: env.grid.key(),
            fit: false,
            fitted: 0,
            hide_deleted: opts.hides_deleted(StoryRef::Body),
            sref: 0,
            typing: None,
        };
        cache.used.insert(key1.clone());
        cache.paras.insert(key1, arc_pl1.clone());

        let mut updated = last_layout.clone();
        let target_page = updated.pages.get_mut(page_idx)?;

        target_page.items[item_idx] = Placed::Lines {
            story: StoryRef::Body,
            path: path0.clone(),
            para: arc_pl0.clone(),
            l0: 0,
            l1: arc_pl0.lines.len(),
            x: item_x,
            y: item_y,
            turn: TextDirection::Horizontal,
        };

        let p1_y = item_y + arc_pl0.height + space_between;
        target_page.items.insert(
            item_idx + 1,
            Placed::Lines {
                story: StoryRef::Body,
                path: path1.clone(),
                para: arc_pl1.clone(),
                l0: 0,
                l1: arc_pl1.lines.len(),
                x: item_x,
                y: p1_y,
                turn: TextDirection::Horizontal,
            },
        );

        for j in (item_idx + 2)..target_page.items.len() {
            match &mut target_page.items[j] {
                Placed::Lines { y, path, .. } => {
                    *y += delta;
                    if let Some(first) = path.0.first_mut() {
                        *first += 1;
                    }
                }
                Placed::Fill { rect, .. }
                | Placed::Image { rect, .. }
                | Placed::Shape { rect, .. }
                | Placed::Graphic { rect, .. }
                | Placed::Clip { rect, .. }
                | Placed::Cell { rect, .. }
                | Placed::Object { rect, .. } => rect.y += delta,
                Placed::Rule { y0, y1, .. } => {
                    *y0 += delta;
                    *y1 += delta;
                }
            }
        }

        let mut new_index = HashMap::with_capacity(updated.index.len() + 1);
        for ((sref, p), list) in updated.index {
            if sref == StoryRef::Body && p.0.len() == 1 {
                let old_idx = p.0[0] as usize;
                if old_idx < split_idx {
                    new_index.insert((sref, p), list);
                } else if old_idx == split_idx {
                    new_index.insert((sref, path0.clone()), vec![(page_idx, item_idx)]);
                    new_index.insert((sref, path1.clone()), vec![(page_idx, item_idx + 1)]);
                } else {
                    let shifted_list: Vec<(usize, usize)> = list
                        .into_iter()
                        .map(|(pi, ii)| if pi == page_idx && ii > item_idx { (pi, ii + 1) } else { (pi, ii) })
                        .collect();
                    new_index.insert((sref, Path::top(old_idx + 1)), shifted_list);
                }
            } else {
                new_index.insert((sref, p), list);
            }
        }
        updated.index = new_index;

        updated.ms = now_ms() - t0;
        cache.last_body = Some(doc.body.clone());
        cache.last_layout = Some(updated.clone());
        return Some(updated);
    }

    // Case 2: Fast-path for Backspace / paragraph merge (doc.body.len() + 1 == last_body.len())
    if doc.body.len() + 1 == last_body.len() {
        let mut merge_idx = None;
        for i in 0..doc.body.len() {
            if !Arc::ptr_eq(&last_body[i], &doc.body[i]) {
                merge_idx = Some(i);
                break;
            }
        }
        let merge_idx = merge_idx.unwrap_or(doc.body.len() - 1);
        for k in (merge_idx + 1)..doc.body.len() {
            if !Arc::ptr_eq(&doc.body[k], &last_body[k + 1]) {
                return None;
            }
        }

        let p0 = doc.body.get(merge_idx)?.as_para()?;
        if !p0.objects.is_empty() || p0.section.is_some() || p0.props.page_break_before.unwrap_or(false) {
            return None;
        }

        let path0 = Path::top(merge_idx);
        let pieces0 = last_layout.index.get(&(StoryRef::Body, path0.clone()))?;
        if pieces0.len() != 1 {
            return None;
        }
        let (page_idx, item_idx0) = pieces0[0];
        if page_idx + 1 != last_layout.pages.len() {
            return None;
        }
        let page = last_layout.pages.get(page_idx)?;
        let item0 = page.items.get(item_idx0)?;
        let Placed::Lines {
            story,
            path: ref item_path0,
            para: ref prev_pl0,
            l0,
            l1,
            x: item_x,
            y: item_y,
            turn,
        } = *item0 else {
            return None;
        };
        if story != StoryRef::Body || *item_path0 != path0 || l0 != 0 || l1 != prev_pl0.lines.len() {
            return None;
        }
        if turn != TextDirection::Horizontal {
            return None;
        }

        let path1 = Path::top(merge_idx + 1);
        let pieces1 = last_layout.index.get(&(StoryRef::Body, path1.clone()))?;
        if pieces1.len() != 1 {
            return None;
        }
        let (page_idx1, item_idx1) = pieces1[0];
        if page_idx1 != page_idx || item_idx1 != item_idx0 + 1 {
            return None;
        }
        let item1 = page.items.get(item_idx1)?;
        let Placed::Lines { para: ref prev_pl1, .. } = *item1 else {
            return None;
        };

        if prev_pl0.has_page_fields
            || !prev_pl0.notes.is_empty()
            || prev_pl0.drop_cap.is_some()
            || prev_pl0.rp.page_break_before
            || prev_pl0.rp.borders.is_some()
            || prev_pl0.rp.shading.is_some()
            || prev_pl0.rp.keep_next
            || prev_pl0.label.is_some()
            || prev_pl1.has_page_fields
            || !prev_pl1.notes.is_empty()
            || prev_pl1.drop_cap.is_some()
            || prev_pl1.rp.page_break_before
            || prev_pl1.rp.borders.is_some()
            || prev_pl1.rp.shading.is_some()
            || prev_pl1.rp.keep_next
            || prev_pl1.label.is_some()
            || p0.props.numbering.is_some()
        {
            return None;
        }

        for j in (item_idx1 + 1)..page.items.len() {
            match &page.items[j] {
                Placed::Lines { story, .. } if *story == StoryRef::Body => {}
                _ => return None,
            }
        }

        let fields = fields::FieldCtx::default();
        let env = para::ParaEnv {
            doc,
            width: col_w,
            label: None,
            fields: &fields,
            show_hidden: opts.show_hidden,
            field_codes: opts.field_codes,
            hide_deleted: opts.hides_deleted(StoryRef::Body),
            table: None,
            proofing: opts.proofing.then_some(opts.proof),
            typing_word: None,
            numeral: opts.numeral,
            exclusions: &[],
            eq_number: 0,
            read_line_spacing: opts.read_line_spacing,
            grid: para::Grid::of(doc, sect),
            fit_objects: false,
            fitted: &[],
        };
        let new_pl0 = para::layout_para(p0, &env);

        let space_between = prev_pl1.rp.space_before.unwrap_or(0.0);
        let delta = new_pl0.height - (prev_pl0.height + space_between + prev_pl1.height);

        if delta > 0.0 {
            let max_bottom = page.items.iter().map(item_bottom).fold(0.0_f32, f32::max);
            if max_bottom + delta > page.body.bottom() + 0.1 {
                return None;
            }
        }

        let arc_pl0 = Arc::new(new_pl0);
        let key0 = cache::Key {
            rev: p0.rev,
            width: col_w.to_bits(),
            label: None,
            page: None,
            page_from: 0,
            table: 0,
            notes: 0,
            excl: 0,
            eq: 0,
            read_spacing: None,
            grid: env.grid.key(),
            fit: false,
            fitted: 0,
            hide_deleted: opts.hides_deleted(StoryRef::Body),
            sref: 0,
            typing: None,
        };
        cache.used.insert(key0.clone());
        cache.paras.insert(key0, arc_pl0.clone());

        let mut updated = last_layout.clone();
        let target_page = updated.pages.get_mut(page_idx)?;

        target_page.items[item_idx0] = Placed::Lines {
            story: StoryRef::Body,
            path: path0.clone(),
            para: arc_pl0.clone(),
            l0: 0,
            l1: arc_pl0.lines.len(),
            x: item_x,
            y: item_y,
            turn: TextDirection::Horizontal,
        };

        target_page.items.remove(item_idx1);

        for j in item_idx1..target_page.items.len() {
            match &mut target_page.items[j] {
                Placed::Lines { y, path, .. } => {
                    *y += delta;
                    if let Some(first) = path.0.first_mut() {
                        *first = first.saturating_sub(1);
                    }
                }
                Placed::Fill { rect, .. }
                | Placed::Image { rect, .. }
                | Placed::Shape { rect, .. }
                | Placed::Graphic { rect, .. }
                | Placed::Clip { rect, .. }
                | Placed::Cell { rect, .. }
                | Placed::Object { rect, .. } => rect.y += delta,
                Placed::Rule { y0, y1, .. } => {
                    *y0 += delta;
                    *y1 += delta;
                }
            }
        }

        let mut new_index = HashMap::with_capacity(updated.index.len());
        for ((sref, p), list) in updated.index {
            if sref == StoryRef::Body && p.0.len() == 1 {
                let old_idx = p.0[0] as usize;
                if old_idx < merge_idx {
                    new_index.insert((sref, p), list);
                } else if old_idx == merge_idx {
                    new_index.insert((sref, path0.clone()), vec![(page_idx, item_idx0)]);
                } else if old_idx == merge_idx + 1 {
                    // Removed block
                } else {
                    let shifted_list: Vec<(usize, usize)> = list
                        .into_iter()
                        .map(|(pi, ii)| if pi == page_idx && ii > item_idx1 { (pi, ii - 1) } else { (pi, ii) })
                        .collect();
                    new_index.insert((sref, Path::top(old_idx - 1)), shifted_list);
                }
            } else {
                new_index.insert((sref, p), list);
            }
        }
        updated.index = new_index;

        updated.ms = now_ms() - t0;
        cache.last_body = Some(doc.body.clone());
        cache.last_layout = Some(updated.clone());
        return Some(updated);
    }

    // Case 3: In-line typing and editing existing line (doc.body.len() == last_body.len())
    if last_body.len() != doc.body.len() {
        return None;
    }

    let mut changed_block = None;
    for (i, (b0, b1)) in last_body.iter().zip(doc.body.iter()).enumerate() {
        if !Arc::ptr_eq(b0, b1) {
            if changed_block.is_none() {
                changed_block = Some(i);
            } else {
                return None;
            }
        }
    }
    let block_idx = changed_block?;
    let block = doc.body.get(block_idx)?;
    let p = block.as_para()?;
    if !p.objects.is_empty() || p.section.is_some() {
        return None;
    }

    let path = Path::top(block_idx);
    let pieces = last_layout.index.get(&(StoryRef::Body, path.clone()))?;
    if pieces.len() != 1 {
        return None;
    }
    let (page_idx, item_idx) = pieces[0];
    let page = last_layout.pages.get(page_idx)?;
    let item = page.items.get(item_idx)?;
    let Placed::Lines {
        story,
        path: ref item_path,
        para: ref prev_pl,
        l0,
        l1,
        x,
        y,
        turn,
    } = *item else {
        return None;
    };
    if story != StoryRef::Body || *item_path != path || l0 != 0 || l1 != prev_pl.lines.len() {
        return None;
    }
    if turn != TextDirection::Horizontal {
        return None;
    }
    if prev_pl.has_page_fields
        || !prev_pl.notes.is_empty()
        || prev_pl.drop_cap.is_some()
        || prev_pl.rp.page_break_before
        || prev_pl.rp.borders.is_some()
        || prev_pl.rp.shading.is_some()
        || prev_pl.rp.keep_next
        || prev_pl.label.is_some()
    {
        return None;
    }

    let col_w = page.body.w;
    let tw = opts.typing_word.as_ref().filter(|t| t.rev == p.rev);
    let typing_range = tw.map(|t| (t.start, t.end));

    let fields = fields::FieldCtx::default();
    let env = para::ParaEnv {
        doc,
        width: col_w,
        label: None,
        fields: &fields,
        show_hidden: opts.show_hidden,
        field_codes: opts.field_codes,
        hide_deleted: opts.hides_deleted(StoryRef::Body),
        table: None,
        proofing: opts.proofing.then_some(opts.proof),
        typing_word: typing_range,
        numeral: opts.numeral,
        exclusions: &[],
        eq_number: 0,
        read_line_spacing: opts.read_line_spacing,
        grid: para::Grid::of(doc, sect),
        fit_objects: false,
        fitted: &[],
    };
    let new_pl = para::layout_para(p, &env);

    let height_diff = (new_pl.height - prev_pl.height).abs();
    let delta = new_pl.height - prev_pl.height;
    let is_last_page = page_idx + 1 == last_layout.pages.len();

    if new_pl.lines.len() != prev_pl.lines.len() || height_diff > 0.01 {
        if !is_last_page {
            return None;
        }
        let max_bottom = page.items.iter().map(item_bottom).fold(0.0_f32, f32::max);
        if max_bottom + delta > page.body.bottom() + 0.1 {
            return None;
        }
    } else {
        for (l_new, l_prev) in new_pl.lines.iter().zip(prev_pl.lines.iter()) {
            if (l_new.height - l_prev.height).abs() > 0.01 || (l_new.top - l_prev.top).abs() > 0.01 {
                if !is_last_page {
                    return None;
                }
            }
        }
    }

    if delta.abs() > 0.001 {
        for j in (item_idx + 1)..page.items.len() {
            match &page.items[j] {
                Placed::Lines { story, .. } if *story == StoryRef::Body => {}
                _ => return None,
            }
        }
    }

    let line_count = new_pl.lines.len();
    let arc_pl = Arc::new(new_pl);
    let key = cache::Key {
        rev: p.rev,
        width: col_w.to_bits(),
        label: None,
        page: None,
        page_from: 0,
        table: 0,
        notes: 0,
        excl: 0,
        eq: 0,
        read_spacing: None,
        grid: env.grid.key(),
        fit: false,
        fitted: 0,
        hide_deleted: opts.hides_deleted(StoryRef::Body),
        sref: 0,
        typing: typing_range,
    };
    cache.used.insert(key.clone());
    cache.paras.insert(key, arc_pl.clone());

    let mut updated = last_layout.clone();
    let target_page = updated.pages.get_mut(page_idx)?;
    let target_item = target_page.items.get_mut(item_idx)?;
    *target_item = Placed::Lines {
        story: StoryRef::Body,
        path,
        para: arc_pl,
        l0: 0,
        l1: line_count,
        x,
        y,
        turn,
    };
    if delta.abs() > 0.001 {
        for j in (item_idx + 1)..target_page.items.len() {
            match &mut target_page.items[j] {
                Placed::Lines { y, .. } => *y += delta,
                Placed::Fill { rect, .. }
                | Placed::Image { rect, .. }
                | Placed::Shape { rect, .. }
                | Placed::Graphic { rect, .. }
                | Placed::Clip { rect, .. }
                | Placed::Cell { rect, .. }
                | Placed::Object { rect, .. } => rect.y += delta,
                Placed::Rule { y0, y1, .. } => {
                    *y0 += delta;
                    *y1 += delta;
                }
            }
        }
    }
    updated.ms = now_ms() - t0;
    cache.last_body = Some(doc.body.clone());
    cache.last_layout = Some(updated.clone());
    Some(updated)
}

/// Lay out the whole document.
pub fn layout(doc: &Document, cache: &mut LayoutCache, opts: &LayoutOptions) -> DocLayout {
    #[cfg(test)]
    crate::test_fonts();
    let t0 = now_ms();
    if let Some(fast) = try_fast_typing_layout(doc, cache, opts, t0) {
        return fast;
    }""",
)

fix_source(
    "crates/layout/src/lib.rs",
    """    l.ms = now_ms() - t0;
    l
}""",
    """    l.ms = now_ms() - t0;
    cache.last_body = Some(doc.body.clone());
    cache.last_layout = Some(l.clone());
    l
}""",
)

if fixed:
    print(f"  Applied typing and large-file performance optimizations for {app}: {', '.join(set(fixed))}")
PY
