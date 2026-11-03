//! What the backend makes of the device a motion event came from.
//!
//! Android describes a device by a bitmask of source flags
//! (`InputDevice.getSources()`) and by the tool that was used with it
//! (`MotionEvent.getToolType()`), and the two have to be read together:
//!
//! * a mouse's source is `SOURCE_MOUSE` and its tool is a mouse;
//! * a touchpad's source is `SOURCE_MOUSE | SOURCE_TOUCHPAD`, but Android documents that
//!   its events "will normally only return `SOURCE_MOUSE`" — the tool is what tells the
//!   two apart: "if it's `TOOL_TYPE_FINGER`, the event is from a touchpad; if it's
//!   `TOOL_TYPE_MOUSE`, the event is from a mouse";
//! * a finger on the glass has a finger's tool too, so the tool on its own means nothing:
//!   it is the source that says whether the finger is on a screen or on a pad.
//!
//! The source is a bitmask and its bits are not exclusive — a touchpad's is a combination
//! of two of them — so a device is recognised by testing the bits and never by matching
//! the whole value against one constant. A value that combines two of them matches no
//! constant at all, and a device that matches nothing is one the backend cannot talk to:
//! its events used to be taken for fingers on the glass, so a trackpad's scroll dragged
//! the document around instead of scrolling it.

/// The type bits of a device's source: they say what kind of device it is.
pub(crate) const TYPE_TOUCHSCREEN: u32 = 0x0000_1000;
pub(crate) const TYPE_MOUSE: u32 = 0x0000_2000;
/// The stylus and the bluetooth stylus, whose type bits overlap.
pub(crate) const TYPE_STYLUS: u32 = 0x0000_c000;
pub(crate) const TYPE_TRACKBALL: u32 = 0x0001_0000;
pub(crate) const TYPE_MOUSE_RELATIVE: u32 = 0x0002_0000;
pub(crate) const TYPE_TOUCHPAD: u32 = 0x0010_0000;

/// The tool a motion event was made with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tool {
    /// The tool is not known: a trackball, for instance, has no tool.
    Unknown,
    /// A finger.
    Finger,
    /// A mouse.
    Mouse,
    /// A stylus.
    Stylus,
    /// Anything else: an eraser, a palm.
    Other,
}

/// What the backend makes of the device a motion event came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DeviceKind {
    /// A pointing device — a mouse, a touchpad, a trackball: its events are translated
    /// into winit's pointer events (hover, click, wheel) and never into touches.
    pub(crate) pointer: bool,
    /// A touchpad: its scroll is smooth and continuous, where a mouse's is a number of
    /// notches.
    pub(crate) touchpad: bool,
}

/// Work out what a device is from its source and the tool it was used with.
pub(crate) fn classify(source: u32, tool: Tool) -> DeviceKind {
    // A finger on the glass and a stylus are never a pointer, whatever else the source
    // says: they draw.
    let on_glass = source & (TYPE_TOUCHSCREEN | TYPE_STYLUS) != 0;
    let pointing = source & (TYPE_MOUSE | TYPE_TRACKBALL | TYPE_MOUSE_RELATIVE | TYPE_TOUCHPAD)
        != 0;
    let pointer = if on_glass {
        false
    } else if pointing {
        true
    } else {
        // A source that says nothing useful — a device Android has added a flag to, say:
        // the tool is the only hint left.
        tool == Tool::Mouse
    };
    // A touchpad is a pointing device whose tool is a finger.
    let touchpad = pointer
        && (source & TYPE_TOUCHPAD != 0
            || (source & TYPE_MOUSE != 0 && tool == Tool::Finger));
    DeviceKind { pointer, touchpad }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The sources Android reports, with their class bits.
    const MOUSE: u32 = 0x0000_2002;
    const MOUSE_RELATIVE: u32 = 0x0002_0004;
    const STYLUS: u32 = 0x0000_4002;
    const BLUETOOTH_STYLUS: u32 = 0x0000_c002;
    const TOUCHSCREEN: u32 = 0x0000_1002;
    const TRACKBALL: u32 = 0x0001_0004;
    const TOUCHPAD: u32 = 0x0010_0008;
    /// A touchpad that also moves the cursor: the two flags together.
    const TOUCHPAD_AND_MOUSE: u32 = TOUCHPAD | MOUSE;

    #[test]
    fn a_mouse_is_a_pointer_and_not_a_touchpad() {
        let kind = classify(MOUSE, Tool::Mouse);
        assert!(kind.pointer);
        assert!(!kind.touchpad);
    }

    #[test]
    fn a_mouse_whose_source_says_nothing_is_still_a_pointer() {
        let kind = classify(0, Tool::Mouse);
        assert!(kind.pointer);
        assert!(!kind.touchpad);
    }

    #[test]
    fn a_touchpad_is_a_pointer_that_scrolls_smoothly() {
        for source in [TOUCHPAD, TOUCHPAD_AND_MOUSE] {
            let kind = classify(source, Tool::Finger);
            assert!(kind.pointer, "source {source:#x}");
            assert!(kind.touchpad, "source {source:#x}");
        }
    }

    #[test]
    fn a_mouse_source_with_a_fingers_tool_is_a_touchpad() {
        // Android documents that a touchpad's events normally report only `SOURCE_MOUSE`
        // and that the tool is what tells it apart from a mouse.
        let kind = classify(MOUSE, Tool::Finger);
        assert!(kind.pointer);
        assert!(kind.touchpad);
    }

    #[test]
    fn a_mouse_source_with_a_mouses_tool_is_not_a_touchpad() {
        let kind = classify(MOUSE, Tool::Mouse);
        assert!(!kind.touchpad);
    }

    #[test]
    fn the_glass_is_never_a_pointer() {
        for source in [TOUCHSCREEN, STYLUS, BLUETOOTH_STYLUS] {
            let kind = classify(source, Tool::Finger);
            assert!(!kind.pointer, "source {source:#x}");
            assert!(!kind.touchpad, "source {source:#x}");
        }
    }

    #[test]
    fn a_screen_that_claims_a_mouses_tool_is_still_a_screen() {
        // The source says what the device is; the tool says what was used with it.
        let kind = classify(TOUCHSCREEN, Tool::Mouse);
        assert!(!kind.pointer);
    }

    #[test]
    fn a_trackball_is_a_pointer_and_not_a_touchpad() {
        let kind = classify(TRACKBALL, Tool::Unknown);
        assert!(kind.pointer);
        assert!(!kind.touchpad);
    }

    #[test]
    fn a_mouse_relative_device_is_a_pointer() {
        let kind = classify(MOUSE_RELATIVE, Tool::Unknown);
        assert!(kind.pointer);
        assert!(!kind.touchpad);
    }

    #[test]
    fn an_unknown_device_with_a_fingers_tool_is_not_a_pointer() {
        let kind = classify(0, Tool::Finger);
        assert!(!kind.pointer);
    }
}
