//! Surface scaling for a 1280 × 720 logical desktop viewport.

const REFERENCE_WIDTH: f64 = 1280.0;
const REFERENCE_HEIGHT: f64 = 720.0;

/// Physical pixels per UI point, invariant under an exchange of the two axes.
///
/// The reference viewport always fits. Any excess space remains available to
/// the application instead of changing the size of its interface elements.
pub(crate) fn pixels_per_point(width: u32, height: u32) -> Option<f64> {
    if width == 0 || height == 0 {
        return None;
    }

    let long_edge = width.max(height) as f64;
    let short_edge = width.min(height) as f64;
    Some((long_edge / REFERENCE_WIDTH).min(short_edge / REFERENCE_HEIGHT))
}
