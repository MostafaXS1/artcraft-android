#!/usr/bin/env bash
# Performance optimizations for WordCraft on Android (large document typing lag).
#
# 1. Fast-path during continuous typing in Session::run:
#    When typing text (self.typing_open is true), avoid cloning the entire 100-step undo
#    history and redo stacks on every keystroke, and skip the full-document block-by-block
#    traversal in unchanged(self).
# 2. Zero-allocation page fingerprinting for fills and rules in page_content_key.
# 3. Pre-allocate line indexing capacity in pagination pass (index_lines).
# 4. Cache comment fonts key so body typing does not scan all comments every keystroke.
# 5. Fast incremental layout during continuous typing in wordcraft_layout::layout:
#    When typing inside an ordinary paragraph without changing its line count or height,
#    re-lay only that single paragraph and reuse the surrounding document page geometry
#    without running full O(N) multi-page pagination across all blocks and sections.
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
        let fast_typing = self.typing_open && !self.history.is_empty() && matches!(id, "text.insert" | "equation.type" | "equation.backspace" | "text.backspace");
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
            let fast_typing = !pushed && self.typing_open && matches!(id, "text.insert" | "equation.type" | "equation.backspace" | "text.backspace");
            let unchanged = |s: &Session| {
                if fast_typing {
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
                    }
                    if !redo.is_empty() {
                        self.redo = redo.clone();
                    }
                    self.typing_open = *typing_open;
                    self.undo_evicted = *evicted;
                }""",
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

# 2. Zero-allocation page fingerprinting for fills and rules
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

# 3. Preallocate line indexing capacity in pagination pass
fix_source(
    "crates/layout/src/pass.rs",
    """fn index_lines(pages: &[Page]) -> HashMap<(StoryRef, Path), Vec<(usize, usize)>> {
    let mut index: HashMap<(StoryRef, Path), Vec<(usize, usize)>> = HashMap::new();""",
    """fn index_lines(pages: &[Page]) -> HashMap<(StoryRef, Path), Vec<(usize, usize)>> {
    let total_placed: usize = pages.iter().map(|p| p.items.len()).sum();
    let mut index: HashMap<(StoryRef, Path), Vec<(usize, usize)>> = HashMap::with_capacity(total_placed / 2);""",
)

# 4. Cache comment fonts key so body typing does not scan all comments every keystroke
fix_source(
    "crates/ui-egui/src/app/fonts.rs",
    """    pub(super) fn ensure_comment_fonts(&mut self) {
        let key = (self.session.document_id(), self.session.rev());
        if self.comment_fonts_for == Some(key) {
            return;
        }
        self.comment_fonts_for = Some(key);""",
    """    pub(super) fn ensure_comment_fonts(&mut self) {
        let comment_hash = {
            let mut h = self.session.doc.comments.len() as u64;
            for (id, c) in &self.session.doc.comments {
                h = h.wrapping_mul(31).wrapping_add(*id as u64).wrapping_add(c.part as u64);
                if let Some(part) = self.session.doc.parts.get(&c.part) {
                    h = h.wrapping_mul(31).wrapping_add(part.blocks.len() as u64);
                }
            }
            h
        };
        let key = (self.session.document_id(), comment_hash);
        if self.comment_fonts_for == Some(key) {
            return;
        }
        self.comment_fonts_for = Some(key);""",
)

# 5. Fast incremental layout during continuous typing in wordcraft_layout::layout:
# When typing inside an ordinary paragraph without changing line count or height,
# re-lay only that single paragraph and reuse the surrounding document page geometry.
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

fix_source(
    "crates/layout/src/lib.rs",
    """/// Lay out the whole document.
pub fn layout(doc: &Document, cache: &mut LayoutCache, opts: &LayoutOptions) -> DocLayout {
    #[cfg(test)]
    crate::test_fonts();
    let t0 = now_ms();""",
    """/// Fast-path incremental layout during continuous typing: when typing inside an ordinary
/// paragraph without altering its line count or height, re-lay only that single paragraph
/// and reuse the surrounding document page geometry.
fn try_fast_typing_layout(
    doc: &Document,
    cache: &mut LayoutCache,
    opts: &LayoutOptions,
    t0: f64,
) -> Option<DocLayout> {
    let tw = opts.typing_word.as_ref()?;
    if opts.view != ViewMode::Print {
        return None;
    }
    let last_layout = cache.last_layout.as_ref()?;
    let last_body = cache.last_body.as_ref()?;
    if last_body.len() != doc.body.len() || last_layout.pages.is_empty() {
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
    if p.rev != tw.rev || !p.objects.is_empty() || p.section.is_some() {
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
        || prev_pl.label.is_some()
    {
        return None;
    }

    let sections = doc.sections();
    let (_, sect) = sections.get(page.section)?;
    let cols = sect.column_boxes();
    if cols.len() != 1 {
        return None;
    }
    let col_w = page.body.w;

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
        typing_word: Some((tw.start, tw.end)),
        numeral: opts.numeral,
        exclusions: &[],
        eq_number: 0,
        read_line_spacing: opts.read_line_spacing,
        grid: para::Grid::of(doc, sect),
        fit_objects: false,
        fitted: &[],
    };
    let new_pl = para::layout_para(p, &env);

    if new_pl.lines.len() != prev_pl.lines.len() || (new_pl.height - prev_pl.height).abs() > 0.01 {
        return None;
    }
    for (l_new, l_prev) in new_pl.lines.iter().zip(prev_pl.lines.iter()) {
        if (l_new.height - l_prev.height).abs() > 0.01 || (l_new.top - l_prev.top).abs() > 0.01 {
            return None;
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
        typing: Some((tw.start, tw.end)),
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
