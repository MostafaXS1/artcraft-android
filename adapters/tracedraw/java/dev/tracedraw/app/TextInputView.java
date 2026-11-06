// The view the on-screen keyboard types into (#1309). NativeActivity
// offers input methods no text field, so a keyboard could only send the
// key presses of Latin letters: Chinese, Japanese, Korean, Tamil and
// every script typed by composing never reached the editor. This view,
// invisible and one pixel large, takes the keyboard's text while the
// editor asks for it (TraceDrawActivity.showTextInput): what the input
// method composes, commits and deletes goes to the editor through the
// activity's native methods (android.rs, soft_keyboard.rs). The keys the
// keyboard presses (Backspace with nothing composed, Enter, the arrows)
// go on to the native input queue as before.
package dev.tracedraw.app;

import android.text.InputType;
import android.util.Log;
import android.view.View;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;

final class TextInputView extends View {
    private final TraceDrawActivity activity;

    TextInputView(TraceDrawActivity activity) {
        super(activity);
        this.activity = activity;
        setFocusable(true);
        setFocusableInTouchMode(true);
    }

    @Override
    public boolean onCheckIsTextEditor() {
        return true;
    }

    @Override
    public InputConnection onCreateInputConnection(EditorInfo out) {
        // Plain text over several lines; the keyboard keeps its own
        // suggestions and transliteration (which compose).
        out.inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE;
        out.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN
                | EditorInfo.IME_FLAG_NO_EXTRACT_UI
                | EditorInfo.IME_ACTION_NONE;
        out.initialSelStart = 0;
        out.initialSelEnd = 0;
        Log.i("tracedraw", "keyboard: the input method connects to the editor");
        return new Connection();
    }

    /**
     * The keyboard's connection. It keeps no text of its own (the editor
     * has the text), so the keyboard sees an empty field: it deletes with
     * Backspace key presses, which reach the editor as keys.
     */
    private final class Connection extends BaseInputConnection {
        /** The text being composed, committed when the keyboard finishes it. */
        private String composing = "";

        Connection() {
            super(TextInputView.this, true);
        }

        @Override
        public boolean setComposingText(CharSequence text, int newCursorPosition) {
            composing = text == null ? "" : text.toString();
            activity.imeCompose(composing);
            return true;
        }

        @Override
        public boolean setComposingRegion(int start, int end) {
            // No text before the caret is known here to compose again.
            return true;
        }

        @Override
        public boolean finishComposingText() {
            if (composing.length() > 0) {
                String text = composing;
                composing = "";
                activity.imeCommit(text);
            }
            return true;
        }

        @Override
        public boolean commitText(CharSequence text, int newCursorPosition) {
            composing = "";
            activity.imeCommit(text == null ? "" : text.toString());
            return true;
        }

        @Override
        public boolean deleteSurroundingText(int beforeLength, int afterLength) {
            activity.imeDelete(beforeLength, afterLength);
            return true;
        }

        @Override
        public boolean deleteSurroundingTextInCodePoints(int beforeLength, int afterLength) {
            activity.imeDelete(beforeLength, afterLength);
            return true;
        }
    }
}
