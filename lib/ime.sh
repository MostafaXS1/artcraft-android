#!/usr/bin/env bash
# The soft keyboard's Backspace at the start of a paragraph — the app-side patch.
#
# The generated launchers and the adapters use android-activity's native IME: the shell polls
# `text_input_state` and turns each change into egui Backspace/Text/Enter events, so Backspace
# normally travels through the same key path as a hardware keyboard and joins paragraphs by
# itself. This script keeps WordCraft's `keys::delete_surrounding` path correct for the cases
# that still use it (an IME that commits through DeleteSurrounding rather than the text state):
# `delete_surrounding` only erases characters inside a paragraph, so Backspace at the paragraph
# start erased nothing and two paragraphs could never be joined on a phone. When the input method
# asks to erase exactly one character before a caret that is already at the start of the
# paragraph, run WordCraft's own `text.backspace` command instead — the same command the
# Backspace key runs on a desktop. Only that case: a larger count is a correction whose text
# really does start at the paragraph start, and must not join paragraphs.
#
# Only WordCraft gets this: the other apps keep their own typing behaviour.
#
# Every edit is guarded and idempotent: it applies only while the original text is still
# present, so it no-ops once the code is there, and it is re-applied on a fresh checkout.
#
# To test WITHOUT the patch, comment out the call to this script in lib/build-common.sh.
#
# Usage: ime.sh <source-dir> <app> <title>
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
    """Replace the single `old` in `rel` with `new`, unless it is already gone or already fixed."""
    path = src / rel
    try:
        text = path.read_text()
    except OSError:
        return
    if old not in text or new in text or text.count(old) != 1:
        return
    path.write_text(text.replace(old, new, 1))
    fixed.append(rel)


# --- Backspace at the start of a paragraph: join it with the one above. ---
#
# Android can deliver one Backspace press as `DeleteSurrounding(1, 0)`, which the app erases with
# `keys::delete_surrounding`. That function only erases characters *inside* a paragraph: its counts
# stop at the paragraph's ends, and when there is nothing to the left it returns without touching the
# document. At the start of a paragraph the one character before the caret is the paragraph mark,
# which has no characters of its own — so Backspace did nothing there, and two paragraphs could never
# be joined: on a phone, once Enter was pressed there was no way to take the line break back.
#
# On a desktop the same key arrives as `Key::Backspace` and the `text.backspace` command joins the
# paragraphs (and, at the start of a list item, removes the number or the indent first, as Word does).
# So when the input method asks to erase exactly one character before a caret that is already at the
# start of the paragraph, run that same command. Only that case: a larger count means a correction
# whose text really does start at the paragraph start, and must not join paragraphs.
fix_source(
    "crates/ui-egui/src/keys.rs",
    """    if start == end {
        return;
    }""",
    """    if start == end {
        // The one character before the caret is the paragraph mark, which the paragraph's text does
        // not contain. Backspace there joins with the paragraph above, exactly as the key does on a
        // desktop; a list item loses its number or indent first. A bigger count is a correction at
        // the paragraph start, not a Backspace over the mark, so it still erases nothing.
        if before == 1 && after == 0 && f.off == 0 {
            if app.session.typing_open() {
                app.session.join_next_undo();
            }
            let _ = app.run("text.backspace", json!({}));
        }
        return;
    }""",
)

# Its regression test: the Backspace-at-the-start-of-a-paragraph case above, and the two cases that
# must keep their old behaviour (the start of the story, and a correction that really starts at the
# paragraph start).
fix_source(
    "crates/ui-egui/src/keys.rs",
    """        alt_frame(&ctx, &mut a, vec![surround(5, 5), egui::Event::Text("2".into())]);
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "one\\n2\\nthree");
    }""",
    """        alt_frame(&ctx, &mut a, vec![surround(5, 5), egui::Event::Text("2".into())]);
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "one\\n2\\nthree");
    }

    /// A phone's Backspace at the start of a paragraph arrives as `DeleteSurrounding(1, 0)`, not as
    /// a `Key::Backspace`. The character it means is the paragraph mark, which is not part of the
    /// paragraph's text, so the paragraphs have to be joined explicitly — otherwise Enter could never
    /// be taken back on a phone.
    #[test]
    fn delete_surrounding_at_a_paragraph_start_joins_it_with_the_one_above() {
        let (ctx, mut a) = alt_frame_app();
        a.run("document.setText", json!({"text": "one\\ntwo"})).unwrap();
        // The caret at the start of `two` (after `one` and its paragraph mark).
        a.run("caret.docStart", json!({})).unwrap();
        for _ in 0..4 {
            a.run("caret.right", json!({})).unwrap();
        }
        alt_frame(&ctx, &mut a, vec![surround(1, 0)]);
        assert_eq!(body(&a), "onetwo");
        // At the very start of the story there is no paragraph above: it still erases nothing.
        a.run("caret.docStart", json!({})).unwrap();
        alt_frame(&ctx, &mut a, vec![surround(1, 0)]);
        assert_eq!(body(&a), "onetwo");
        // A correction erasing several characters from the paragraph start is not a Backspace over
        // the mark, so it must not join the paragraphs.
        a.run("document.setText", json!({"text": "one\\ntwo"})).unwrap();
        a.run("caret.docStart", json!({})).unwrap();
        for _ in 0..4 {
            a.run("caret.right", json!({})).unwrap();
        }
        alt_frame(&ctx, &mut a, vec![surround(2, 0), egui::Event::Text("X".into())]);
        assert_eq!(body(&a), "one\\nXtwo");
    }""",
)

if fixed:
    print(f"  Patched the soft keyboard for {app}: {', '.join(fixed)}")
PY
