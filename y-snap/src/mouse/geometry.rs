//! Small rectangle/point helpers shared by [`super::gesture`] and
//! [`super::hover`] — neither concern owns these, both need them.

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
use windows::core::Result;

pub fn window_rect(window: HWND) -> Result<RECT> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(window, &mut rect) }?;
    Ok(rect)
}

pub fn point_in_window(window: HWND, pt: POINT) -> bool {
    window_rect(window).is_ok_and(|rect| point_in(rect, pt))
}

pub fn point_in(rect: RECT, pt: POINT) -> bool {
    pt.x >= rect.left && pt.x < rect.right && pt.y >= rect.top && pt.y < rect.bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_is_inside_its_own_rectangle_but_not_past_its_edges() {
        let rect = RECT {
            left: 10,
            top: 10,
            right: 20,
            bottom: 20,
        };
        assert!(point_in(rect, POINT { x: 10, y: 10 })); // top-left is inside
        assert!(!point_in(rect, POINT { x: 20, y: 15 })); // right edge is not
        assert!(!point_in(rect, POINT { x: 15, y: 20 })); // bottom edge is not
        assert!(!point_in(rect, POINT { x: 9, y: 15 }));
    }
}
