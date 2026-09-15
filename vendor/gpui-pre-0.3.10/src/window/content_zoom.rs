use super::{App, DispatchEventResult, FileDropEvent, PlatformInput, Window};
use crate::ScrollDelta;

impl Window {
    /// Application content zoom, independent of the operating system's display density.
    pub fn content_zoom(&self) -> f32 {
        self.content_zoom
    }

    /// Scales the entire content coordinate system, including fixed pixel sizes.
    /// Native window bounds stay in platform coordinates. Invalid factors are ignored.
    pub fn set_content_zoom(&mut self, zoom: f32) {
        if !zoom.is_finite() || zoom <= 0. || zoom == self.content_zoom {
            return;
        }
        let ratio = self.content_zoom / zoom;
        self.viewport_size = self.platform_window.content_size() / zoom;
        self.mouse_position = self.mouse_position * ratio;
        self.content_zoom = zoom;
        self.refresh();
    }

    // Only platform input crosses this boundary. Synthetic events and test helpers
    // already use content coordinates and continue through dispatch_event directly.
    pub(super) fn dispatch_platform_event(
        &mut self,
        mut event: PlatformInput,
        cx: &mut App,
    ) -> DispatchEventResult {
        let zoom = self.content_zoom;
        match &mut event {
            PlatformInput::MouseDown(e) => e.position = e.position / zoom,
            PlatformInput::MouseUp(e) => e.position = e.position / zoom,
            PlatformInput::MouseMove(e) => e.position = e.position / zoom,
            PlatformInput::MousePressure(e) => e.position = e.position / zoom,
            PlatformInput::MouseExited(e) => e.position = e.position / zoom,
            PlatformInput::Scroll(e) => {
                e.position = e.position / zoom;
                if let ScrollDelta::Pixels(delta) = &mut e.delta {
                    *delta = *delta / zoom;
                }
            }
            PlatformInput::Pinch(e) => e.position = e.position / zoom,
            PlatformInput::Touch(e) => {
                e.position = e.position / zoom;
                e.predicted_position = e.predicted_position.map(|p| p / zoom);
            }
            PlatformInput::LongPress(e) => {
                e.position = e.position / zoom;
                e.start_position = e.start_position / zoom;
            }
            PlatformInput::TouchDrag(e) => {
                e.position = e.position / zoom;
                e.start_position = e.start_position / zoom;
            }
            PlatformInput::BackGesture(e) => e.start_position = e.start_position / zoom,
            PlatformInput::FileDrop(
                FileDropEvent::Entered { position, .. }
                | FileDropEvent::Pending { position }
                | FileDropEvent::Submit { position },
            ) => *position = *position / zoom,
            _ => {}
        }
        self.dispatch_event(event, cx)
    }
}
