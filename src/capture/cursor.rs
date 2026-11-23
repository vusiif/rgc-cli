use mouse_position::mouse_position::Mouse;

use crate::api::CursorInfo;

/// Retrieve the current global OS desktop coordinates of the mouse cursor.
/// Returns `None` if the platform or environment cannot query the cursor position.
pub fn get_global_cursor_position() -> Option<(i32, i32)> {
    match Mouse::get_mouse_position() {
        Mouse::Position { x, y } => Some((x, y)),
        Mouse::Error => None,
    }
}

/// Project global cursor position onto a captured canvas given monitor/window origin,
/// DPI scale factor, and crop offsets.
pub fn resolve_cursor_on_canvas(
    cursor_global: Option<(i32, i32)>,
    origin: (i32, i32),
    scale_factor: f32,
    crop_offset: (u32, u32),
    canvas_size: (u32, u32),
) -> Option<CursorInfo> {
    let (gx, gy) = cursor_global?;
    let (ox, oy) = origin;
    let (crop_x, crop_y) = crop_offset;
    let (cw, ch) = canvas_size;

    let rel_x = gx.saturating_sub(ox);
    let rel_y = gy.saturating_sub(oy);

    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor as f64
    } else {
        1.0
    };

    let px = (rel_x as f64 * scale).round() as i32 - crop_x as i32;
    let py = (rel_y as f64 * scale).round() as i32 - crop_y as i32;

    let visible = px >= 0 && px < cw as i32 && py >= 0 && py < ch as i32;

    Some(CursorInfo {
        x: px,
        y: py,
        visible,
        screen_x: Some(gx),
        screen_y: Some(gy),
        canvas_x: Some(px),
        canvas_y: Some(py),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_cursor_on_canvas_inside() {
        let resolved =
            resolve_cursor_on_canvas(Some((500, 300)), (100, 100), 1.0, (0, 0), (1920, 1080));
        assert_eq!(
            resolved,
            Some(CursorInfo {
                x: 400,
                y: 200,
                visible: true,
                screen_x: Some(500),
                screen_y: Some(300),
                canvas_x: Some(400),
                canvas_y: Some(200),
            })
        );
    }

    #[test]
    fn test_resolve_cursor_on_canvas_scaled_and_cropped() {
        let resolved =
            resolve_cursor_on_canvas(Some((300, 200)), (100, 100), 1.5, (50, 20), (800, 600));
        assert_eq!(
            resolved,
            Some(CursorInfo {
                x: 250,
                y: 130,
                visible: true,
                screen_x: Some(300),
                screen_y: Some(200),
                canvas_x: Some(250),
                canvas_y: Some(130),
            })
        );
    }

    #[test]
    fn test_resolve_cursor_on_canvas_outside() {
        let resolved =
            resolve_cursor_on_canvas(Some((50, 50)), (100, 100), 1.0, (0, 0), (800, 600));
        assert_eq!(
            resolved,
            Some(CursorInfo {
                x: -50,
                y: -50,
                visible: false,
                screen_x: Some(50),
                screen_y: Some(50),
                canvas_x: Some(-50),
                canvas_y: Some(-50),
            })
        );
    }

    #[test]
    fn test_resolve_cursor_none() {
        assert_eq!(
            resolve_cursor_on_canvas(None, (0, 0), 1.0, (0, 0), (100, 100)),
            None
        );
    }
}
