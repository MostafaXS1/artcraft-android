#!/usr/bin/env bash
# WordCraft typing lag and large-file performance optimizations.
#
# When typing in large documents (hundreds of pages, thousands of blocks):
# 1. Speculative rollback snapshots cloned the entire undo history (up to 100 snapshots
#    of every document block and style) and redo queue on every single character typed.
#    During continuous typing (joined undo checkpoint), history and redo are unchanged,
#    so cloning them can be safely skipped.
# 2. On every keystroke, `unchanged` traversed thousands of blocks comparing `same_content`
#    even though text was just inserted and definitely changed. Fast-pathing joined typing
#    avoids traversing the whole document.
# 3. Page fingerprinting (`page_content_key`) heap-allocated formatted debug strings for
#    every fill and rule on each page. Direct hashing removes this heap churn.
# 4. `index_lines` in pagination preallocates HashMap capacity to prevent repeated map growth.
#
# Only WordCraft gets this: the other apps keep their own behaviour.
#
# Every edit is guarded and idempotent: it applies only while the original text is still
# present, so it no-ops once the code is there, and it is re-applied on a fresh checkout.
#
# Usage: wordcraft_opt.sh <source-dir> <app> <title>
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

def fix_source(rel: str, old: str, new: str) -> bool:
    """Replace the single `old` in `rel` with `new`, unless it is already gone or already fixed."""
    path = src / rel
    try:
        text = path.read_text()
    except OSError:
        return False
    if old not in text or new in text or text.count(old) != 1:
        return False
    path.write_text(text.replace(old, new, 1))
    fixed.append(rel)
    return True

# 1. Fast-path undo history and unchanged checking during continuous typing in run.rs
fix_source(
    "crates/engine/src/session/run.rs",
    """        // Commands may leave an equation (a whole-body replacement) even after they fail; put
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
    """        // Commands may leave an equation (a whole-body replacement) even after they fail; put
        // the editing session back with the document.
        let math_before = spec.mutates.then(|| self.math.clone());
        let fast_typing = join && self.join_anchor && !self.history.is_empty() && matches!(id, "text.insert" | "equation.type" | "equation.backspace" | "text.backspace");
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
    """            let unchanged = |s: &Session| match (before_doc, pending_before) {
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
    """            let fast_typing = !pushed && self.typing_open && matches!(id, "text.insert" | "equation.type" | "equation.backspace" | "text.backspace");
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

if fixed:
    print(f"  Applied typing and large-file performance optimizations for {app}: {', '.join(set(fixed))}")
PY
