#![forbid(unsafe_code)]

//! Accessibility tree query and spatial fusion with Set-of-Mark (SoM) vision detection.
//!
//! Bridges visual perception with OS accessibility semantics:
//! 1. Represents structured accessibility nodes (role, label, state, screen bounding box).
//! 2. Performs geometric spatial fusion (IoU and containment matching) to associate
//!    visual element bounding boxes with OS accessibility semantic labels (e.g. "Button Save").
//! 3. Injects interactive accessibility controls that lack distinct visual borders.

use serde::{Deserialize, Serialize};

use crate::api::UiElement;
use crate::coords::CoordinateTransform;

/// Normalized accessibility node representing an OS desktop UI element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityNode {
    /// Screen desktop X coordinate.
    pub screen_x: i32,
    /// Screen desktop Y coordinate.
    pub screen_y: i32,
    /// Width in screen pixels.
    pub width: u32,
    /// Height in screen pixels.
    pub height: u32,
    /// Control role (e.g., "Button", "Edit", "ComboBox", "CheckBox", "RadioButton", "MenuItem", "Hyperlink", "TabItem", "Window").
    pub role: String,
    /// Human-readable name or label (e.g., "Save", "Submit", "Search").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Whether the control is currently enabled for interaction.
    pub enabled: bool,
}

impl AccessibilityNode {
    pub fn new(
        screen_x: i32,
        screen_y: i32,
        width: u32,
        height: u32,
        role: impl Into<String>,
        label: Option<String>,
        enabled: bool,
    ) -> Self {
        Self {
            screen_x,
            screen_y,
            width,
            height,
            role: role.into(),
            label,
            enabled,
        }
    }

    /// Center X coordinate in screen desktop space.
    pub fn center_x(&self) -> i32 {
        self.screen_x + (self.width as i32) / 2
    }

    /// Center Y coordinate in screen desktop space.
    pub fn center_y(&self) -> i32 {
        self.screen_y + (self.height as i32) / 2
    }

    /// Bounding rectangle tuple (x, y, width, height).
    pub fn bounds(&self) -> (i32, i32, u32, u32) {
        (self.screen_x, self.screen_y, self.width, self.height)
    }

    /// Whether this node represents a typical interactive control eligible for click automation.
    pub fn is_interactive(&self) -> bool {
        let role = self.role.to_lowercase();
        matches!(
            role.as_str(),
            "button"
                | "edit"
                | "textbox"
                | "combobox"
                | "checkbox"
                | "radiobutton"
                | "menuitem"
                | "hyperlink"
                | "link"
                | "tabitem"
                | "splitbutton"
                | "slider"
                | "toggle"
        )
    }
}

/// Calculate Intersection-over-Union (IoU) of two bounding rectangles in the same coordinate space.
pub fn calculate_rect_iou(r1: (i32, i32, u32, u32), r2: (i32, i32, u32, u32)) -> f32 {
    let (x1, y1, w1, h1) = r1;
    let (x2, y2, w2, h2) = r2;

    if w1 == 0 || h1 == 0 || w2 == 0 || h2 == 0 {
        return 0.0;
    }

    let left = x1.max(x2);
    let top = y1.max(y2);
    let right = (x1 + w1 as i32).min(x2 + w2 as i32);
    let bottom = (y1 + h1 as i32).min(y2 + h2 as i32);

    if right <= left || bottom <= top {
        return 0.0;
    }

    let intersection_area = ((right - left) as f64) * ((bottom - top) as f64);
    let area1 = (w1 as f64) * (h1 as f64);
    let area2 = (w2 as f64) * (h2 as f64);
    let union_area = area1 + area2 - intersection_area;

    if union_area <= 0.0 {
        0.0
    } else {
        (intersection_area / union_area) as f32
    }
}

/// Check if a 2D point is contained within a rectangle.
pub fn point_in_rect(px: i32, py: i32, rect: (i32, i32, u32, u32)) -> bool {
    let (rx, ry, rw, rh) = rect;
    px >= rx && px < (rx + rw as i32) && py >= ry && py < (ry + rh as i32)
}

/// Calculate what percentage of `inner` is covered by `outer`.
pub fn containment_ratio(inner: (i32, i32, u32, u32), outer: (i32, i32, u32, u32)) -> f32 {
    let (ix, iy, iw, ih) = inner;
    let (ox, oy, ow, oh) = outer;

    if iw == 0 || ih == 0 || ow == 0 || oh == 0 {
        return 0.0;
    }

    let left = ix.max(ox);
    let top = iy.max(oy);
    let right = (ix + iw as i32).min(ox + ow as i32);
    let bottom = (iy + ih as i32).min(oy + oh as i32);

    if right <= left || bottom <= top {
        return 0.0;
    }

    let intersection_area = ((right - left) as f64) * ((bottom - top) as f64);
    let inner_area = (iw as f64) * (ih as f64);

    (intersection_area / inner_area) as f32
}

/// Fuse detected visual UI elements with accessibility nodes.
///
/// Modifies visual elements in place by attaching `role`, `label`, and `enabled` attributes.
/// If `inject_missing` is true, high-value interactive accessibility controls that were
/// missed by edge/visual clustering are transformed into image space and appended as `UiElement`.
///
/// Returns the number of fused and injected elements.
pub fn fuse_accessibility_and_vision(
    elements: &mut Vec<UiElement>,
    a11y_nodes: &[AccessibilityNode],
    transform: &CoordinateTransform,
    inject_missing: bool,
) -> usize {
    if a11y_nodes.is_empty() {
        return 0;
    }

    let mut matched_nodes = vec![false; a11y_nodes.len()];
    let mut fused_count = 0;

    // 1. Associate each visual element with the closest matching accessibility node in screen space
    for elem in elements.iter_mut() {
        let elem_screen_cx = match elem.screen_center_x {
            Some(cx) => cx,
            None => {
                let (cx, _) = transform.image_to_screen(elem.center_x, elem.center_y);
                cx
            }
        };
        let elem_screen_cy = match elem.screen_center_y {
            Some(cy) => cy,
            None => {
                let (_, cy) = transform.image_to_screen(elem.center_x, elem.center_y);
                cy
            }
        };

        let (screen_tl_x, screen_tl_y) = transform.image_to_screen(elem.x, elem.y);
        let (screen_br_x, screen_br_y) =
            transform.image_to_screen(elem.x + elem.width, elem.y + elem.height);

        let elem_screen_w = (screen_br_x - screen_tl_x).unsigned_abs();
        let elem_screen_h = (screen_br_y - screen_tl_y).unsigned_abs();
        let elem_rect = (screen_tl_x, screen_tl_y, elem_screen_w, elem_screen_h);

        let mut best_score = 0.0f32;
        let mut best_idx = None;

        for (idx, node) in a11y_nodes.iter().enumerate() {
            let node_rect = node.bounds();
            let iou = calculate_rect_iou(elem_rect, node_rect);
            let contains_center = point_in_rect(elem_screen_cx, elem_screen_cy, node_rect);
            let contain_ratio = containment_ratio(elem_rect, node_rect);

            // Compute composite match score
            let mut score = iou * 0.6;
            if contains_center {
                score += 0.25;
            }
            if contain_ratio > 0.6 {
                score += 0.15;
            }

            // Prefer interactive controls over giant root windows or background frames
            if node.is_interactive() {
                score += 0.1;
            }

            if score > best_score && score >= 0.20 {
                best_score = score;
                best_idx = Some(idx);
            }
        }

        if let Some(idx) = best_idx {
            let node = &a11y_nodes[idx];
            elem.role = Some(node.role.clone());
            elem.label = node.label.clone();
            elem.enabled = Some(node.enabled);
            elem.source = Some("hybrid".to_string());
            matched_nodes[idx] = true;
            fused_count += 1;
        }
    }

    // 2. Optionally inject unmatched interactive accessibility nodes into the UiElement set
    if inject_missing {
        let (output_w, output_h) = transform.output_size;
        let max_id = elements.iter().map(|e| e.id).max().unwrap_or(0);
        let mut next_id = max_id + 1;

        for (idx, node) in a11y_nodes.iter().enumerate() {
            if matched_nodes[idx] || !node.is_interactive() {
                continue;
            }

            // Check if node is large enough to be an interactive target
            if node.width < 8 || node.height < 8 {
                continue;
            }

            // Map screen coordinates back to image space
            let (img_tl_x, img_tl_y) = transform.screen_to_image(node.screen_x, node.screen_y);
            let (img_br_x, img_br_y) = transform.screen_to_image(
                node.screen_x + node.width as i32,
                node.screen_y + node.height as i32,
            );

            // Clamp to output image boundary
            let img_x0 = img_tl_x.clamp(0, output_w as i32) as u32;
            let img_y0 = img_tl_y.clamp(0, output_h as i32) as u32;
            let img_x1 = img_br_x.clamp(0, output_w as i32) as u32;
            let img_y1 = img_br_y.clamp(0, output_h as i32) as u32;

            if img_x1 <= img_x0 || img_y1 <= img_y0 {
                continue;
            }

            let width = img_x1 - img_x0;
            let height = img_y1 - img_y0;

            if width < 6 || height < 6 {
                continue;
            }

            let mut injected = UiElement::new(next_id, img_x0, img_y0, width, height);
            injected.role = Some(node.role.clone());
            injected.label = node.label.clone();
            injected.enabled = Some(node.enabled);
            injected.source = Some("accessibility".to_string());

            let (sc_x, sc_y) = transform.image_to_screen(injected.center_x, injected.center_y);
            let (st_x, st_y) = transform.image_to_screen(injected.x, injected.y);
            injected.screen_center_x = Some(sc_x);
            injected.screen_center_y = Some(sc_y);
            injected.screen_x = Some(st_x);
            injected.screen_y = Some(st_y);

            elements.push(injected);
            next_id += 1;
            fused_count += 1;
        }
    }

    fused_count
}

/// Retrieve system accessibility nodes intersecting with the target screen capture area.
///
/// On Windows, queries active desktop window hierarchy to extract top-level UI controls.
/// Safely falls back to an empty list on platforms or environments without active accessibility providers.
pub fn query_system_accessibility_nodes(
    screen_bounds: Option<(i32, i32, u32, u32)>,
) -> Vec<AccessibilityNode> {
    let mut nodes = Vec::new();

    // Query window list from xcap to extract basic window-level accessibility semantics
    if let Ok(windows) = crate::capture::window::get_windows() {
        for w in windows {
            let title = w.title().unwrap_or_default();
            let app_name = w.app_name().unwrap_or_default();
            let x = w.x().unwrap_or(0);
            let y = w.y().unwrap_or(0);
            let width = w.width().unwrap_or(0);
            let height = w.height().unwrap_or(0);
            let is_minimized = w.is_minimized().unwrap_or(false);

            if is_minimized
                || width == 0
                || height == 0
                || (title.is_empty() && app_name.is_empty())
            {
                continue;
            }

            // Filter by screen capture area if bounded
            if let Some((sx, sy, sw, sh)) = screen_bounds {
                if calculate_rect_iou((x, y, width, height), (sx, sy, sw, sh)) <= 0.0 {
                    continue;
                }
            }

            let label = if !title.is_empty() {
                Some(title)
            } else {
                Some(app_name)
            };

            nodes.push(AccessibilityNode::new(
                x, y, width, height, "Window", label, true,
            ));
        }
    }

    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_rect_iou() {
        // Completely identical boxes
        let r1 = (100, 100, 50, 50);
        let r2 = (100, 100, 50, 50);
        assert!((calculate_rect_iou(r1, r2) - 1.0).abs() < 1e-4);

        // Completely disjoint boxes
        let r3 = (200, 200, 50, 50);
        assert_eq!(calculate_rect_iou(r1, r3), 0.0);

        // Half overlap along X
        let r4 = (125, 100, 50, 50);
        let iou = calculate_rect_iou(r1, r4);
        // intersection = 25 * 50 = 1250, union = 2500 + 2500 - 1250 = 3750, iou = 1250/3750 = 0.3333
        assert!((iou - 0.3333).abs() < 1e-3);
    }

    #[test]
    fn test_point_in_rect() {
        let rect = (50, 50, 100, 100);
        assert!(point_in_rect(50, 50, rect));
        assert!(point_in_rect(100, 100, rect));
        assert!(point_in_rect(149, 149, rect));
        assert!(!point_in_rect(49, 50, rect));
        assert!(!point_in_rect(150, 150, rect));
    }

    #[test]
    fn test_containment_ratio() {
        let inner = (10, 10, 20, 20); // area 400
        let outer = (0, 0, 100, 100); // contains inner completely
        assert!((containment_ratio(inner, outer) - 1.0).abs() < 1e-4);

        let outer_half = (10, 10, 10, 20); // overlaps half of inner (area 200)
        assert!((containment_ratio(inner, outer_half) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn test_fuse_accessibility_and_vision() {
        let transform = CoordinateTransform::identity((1920, 1080));

        // Create visual elements
        let mut elements = vec![
            UiElement::new(1, 100, 100, 80, 30),  // Vision button
            UiElement::new(2, 500, 500, 200, 40), // Vision input
        ];

        // Ensure screen coordinates are set
        for e in &mut elements {
            e.screen_center_x = Some(e.center_x as i32);
            e.screen_center_y = Some(e.center_y as i32);
            e.screen_x = Some(e.x as i32);
            e.screen_y = Some(e.y as i32);
        }

        // Accessibility nodes
        let a11y_nodes = vec![
            AccessibilityNode::new(98, 98, 84, 34, "Button", Some("Submit".to_string()), true),
            AccessibilityNode::new(
                495,
                495,
                210,
                45,
                "Edit",
                Some("Username".to_string()),
                true,
            ),
            AccessibilityNode::new(800, 800, 60, 30, "Button", Some("Cancel".to_string()), true), // Missed by vision
        ];

        let count = fuse_accessibility_and_vision(&mut elements, &a11y_nodes, &transform, true);
        assert_eq!(count, 3); // 2 fused + 1 injected
        assert_eq!(elements.len(), 3);

        // Verify fused element 1
        assert_eq!(elements[0].role.as_deref(), Some("Button"));
        assert_eq!(elements[0].label.as_deref(), Some("Submit"));
        assert_eq!(elements[0].source.as_deref(), Some("hybrid"));

        // Verify fused element 2
        assert_eq!(elements[1].role.as_deref(), Some("Edit"));
        assert_eq!(elements[1].label.as_deref(), Some("Username"));
        assert_eq!(elements[1].source.as_deref(), Some("hybrid"));

        // Verify injected element 3
        assert_eq!(elements[2].id, 3);
        assert_eq!(elements[2].role.as_deref(), Some("Button"));
        assert_eq!(elements[2].label.as_deref(), Some("Cancel"));
        assert_eq!(elements[2].source.as_deref(), Some("accessibility"));
    }
}
