use gtk::{graphene::Rect, prelude::*};

use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use crate::highlight::HighlightMap;
use crate::mode;
use crate::nvim::RedrawMode;
use crate::render;
use crate::render::CellMetrics;
use crate::ui::UiMutex;
use crate::ui_model::Cell;

struct Alpha(f64);

impl Alpha {
    pub fn show(&mut self, step: f64) -> bool {
        self.0 += step;
        if self.0 > 1.0 {
            self.0 = 1.0;
            false
        } else {
            true
        }
    }
    pub fn hide(&mut self, step: f64) -> bool {
        self.0 -= step;
        if self.0 < 0.0 {
            self.0 = 0.0;
            false
        } else {
            true
        }
    }
}

#[derive(PartialEq)]
enum AnimPhase {
    Shown,
    Hide,
    Hidden,
    Show,
    NoFocus,
    Busy,
}

struct BlinkCount {
    count: u32,
    max: u32,
}

impl BlinkCount {
    fn new(max: u32) -> Self {
        Self { count: 0, max }
    }
}

struct State<CB: CursorRedrawCb> {
    alpha: Alpha,
    anim_phase: AnimPhase,
    redraw_cb: Weak<UiMutex<CB>>,

    timer: Option<glib::SourceId>,
    counter: Option<BlinkCount>,
    widget_focus: bool,
    toplevel_focus: bool,
}

impl<CB: CursorRedrawCb> State<CB> {
    fn new(redraw_cb: Weak<UiMutex<CB>>) -> Self {
        State {
            alpha: Alpha(1.0),
            anim_phase: AnimPhase::Shown,
            redraw_cb,
            timer: None,
            counter: None,
            widget_focus: false,
            toplevel_focus: false,
        }
    }

    fn reset_to(&mut self, phase: AnimPhase) {
        self.alpha = Alpha(1.0);
        self.anim_phase = phase;
        if let Some(timer_id) = self.timer.take() {
            timer_id.remove();
        }
    }

    fn focus(&self) -> bool {
        self.toplevel_focus && self.widget_focus
    }
}

pub struct Cursor<CB: CursorRedrawCb> {
    state: Arc<UiMutex<State<CB>>>,
    mode_info: Option<mode::ModeInfo>,
}

impl<CB: CursorRedrawCb + 'static> Cursor<CB> {
    pub fn new(redraw_cb: Weak<UiMutex<CB>>) -> Self {
        Cursor {
            state: Arc::new(UiMutex::new(State::new(redraw_cb))),
            mode_info: None,
        }
    }

    pub fn set_mode_info(&mut self, mode_info: Option<mode::ModeInfo>) {
        self.mode_info = mode_info;
    }

    pub fn set_cursor_blink(&mut self, val: i32) {
        let mut mut_state = self.state.borrow_mut();
        mut_state.counter = if val < 0 {
            None
        } else {
            Some(BlinkCount::new(val as u32))
        }
    }

    pub fn start(&mut self) {
        let blinkwait = self
            .mode_info
            .as_ref()
            .and_then(|mi| mi.blinkwait)
            .unwrap_or(500);

        let state = self.state.clone();
        let mut mut_state = self.state.borrow_mut();

        mut_state.reset_to(AnimPhase::Shown);

        if let Some(counter) = &mut mut_state.counter {
            counter.count = 0;
        }

        mut_state.timer = Some(glib::timeout_add(
            Duration::from_millis(if blinkwait > 0 { blinkwait as u64 } else { 500 }),
            move || anim_step(&state),
        ));
    }

    pub fn reset_state(&mut self) {
        if self.state.borrow().anim_phase != AnimPhase::Busy {
            self.start();
        }
    }

    fn update_focus(&mut self, focus: bool) {
        if self.state.borrow().anim_phase != AnimPhase::Busy {
            if focus {
                self.start();
            } else {
                self.state.borrow_mut().reset_to(AnimPhase::NoFocus);
            }
        }
    }

    #[must_use]
    pub fn set_toplevel_focus(&mut self, focus: bool) -> RedrawMode {
        let mut state = self.state.borrow_mut();
        let prev_focus = state.focus();

        state.toplevel_focus = focus;
        if prev_focus != state.focus() {
            drop(state);
            self.update_focus(focus);
            RedrawMode::Cursor
        } else {
            RedrawMode::Nothing
        }
    }

    #[must_use]
    pub fn set_widget_focus(&mut self, focus: bool) -> RedrawMode {
        let mut state = self.state.borrow_mut();
        let prev_focus = state.focus();

        state.widget_focus = focus;
        if prev_focus != state.focus() {
            drop(state);
            self.update_focus(focus);
            RedrawMode::Cursor
        } else {
            RedrawMode::Nothing
        }
    }

    pub fn busy_on(&mut self) {
        self.state.borrow_mut().reset_to(AnimPhase::Busy);
    }

    pub fn busy_off(&mut self) {
        self.start();
    }

    /// Add render nodes for the cursor to the snapshot. Returns whether or not text should be drawn
    /// after
    pub fn snapshot(
        &self,
        snapshot: &gtk::Snapshot,
        font_ctx: &render::Context,
        (x, y): (f64, f64),
        cell: &Cell,
        double_width: bool,
        hl: &HighlightMap,
        fade_percentage: f64,
        filled_alpha: f64,
    ) -> bool {
        let state = self.state.borrow();

        let cell_metrics = font_ctx.cell_metrics();
        let (y, w, h) = cursor_rect(self.mode_info(), cell_metrics, y, double_width);
        let (x, y, w, h) = (x as f32, y as f32, w as f32, h as f32);

        if state.anim_phase == AnimPhase::NoFocus {
            #[rustfmt::skip]
            {
                let bg = hl.cursor_bg().to_rgbo(filled_alpha);
                snapshot.append_color(&bg, &Rect::new(          x,           y,   w, 1.0));
                snapshot.append_color(&bg, &Rect::new(          x,           y, 1.0,   h));
                snapshot.append_color(&bg, &Rect::new(          x, y + h - 1.0,   w, 1.0));
                snapshot.append_color(&bg, &Rect::new(x + w - 1.0,           y, 1.0,   h));
            };
            false
        } else {
            let bg = hl
                .actual_cell_bg(cell)
                .fade(&hl.cursor_bg(), fade_percentage)
                .as_ref()
                .to_rgbo(filled_alpha);
            snapshot.append_color(&bg, &Rect::new(x, y, w, h));
            true
        }
    }

    pub fn alpha(&self) -> f64 {
        self.state.borrow().alpha.0
    }

    pub fn is_visible(&self) -> bool {
        let state = self.state.borrow();

        if state.anim_phase == AnimPhase::Busy {
            return false;
        }

        state.alpha.0 >= 0.000001
    }

    pub fn is_focused(&self) -> bool {
        self.state.borrow().focus()
    }

    pub fn mode_info(&self) -> Option<&mode::ModeInfo> {
        self.mode_info.as_ref()
    }
}

pub fn cursor_rect(
    mode_info: Option<&mode::ModeInfo>,
    cell_metrics: &CellMetrics,
    line_y: f64,
    double_width: bool,
) -> (f64, f64, f64) {
    let &CellMetrics {
        line_height,
        char_width,
        ..
    } = cell_metrics;

    if let Some(mode_info) = mode_info {
        match mode_info.cursor_shape() {
            None | Some(&mode::CursorShape::Unknown) | Some(&mode::CursorShape::Block) => {
                let cursor_width = if double_width {
                    char_width * 2.0
                } else {
                    char_width
                };
                (line_y, cursor_width, line_height)
            }
            Some(&mode::CursorShape::Vertical) => {
                let cell_percentage = mode_info.cell_percentage();
                let cursor_width = if cell_percentage > 0 {
                    (char_width * cell_percentage as f64) / 100.0
                } else {
                    char_width
                };
                (line_y, cursor_width, line_height)
            }
            Some(&mode::CursorShape::Horizontal) => {
                let cell_percentage = mode_info.cell_percentage();
                let cursor_width = if double_width {
                    char_width * 2.0
                } else {
                    char_width
                };

                if cell_percentage > 0 {
                    let height = (line_height * cell_percentage as f64) / 100.0;
                    (line_y + line_height - height, cursor_width, height)
                } else {
                    (line_y, cursor_width, line_height)
                }
            }
        }
    } else {
        let cursor_width = if double_width {
            char_width * 2.0
        } else {
            char_width
        };

        (line_y, cursor_width, line_height)
    }
}

fn anim_step<CB: CursorRedrawCb + 'static>(state: &Arc<UiMutex<State<CB>>>) -> glib::ControlFlow {
    let mut mut_state = state.borrow_mut();

    let next_event = match mut_state.anim_phase {
        AnimPhase::Shown => {
            if let Some(counter) = &mut mut_state.counter {
                if counter.count < counter.max {
                    counter.count += 1;
                    mut_state.anim_phase = AnimPhase::Hide;
                    Some(60)
                } else {
                    None
                }
            } else {
                mut_state.anim_phase = AnimPhase::Hide;
                Some(60)
            }
        }
        AnimPhase::Hide => {
            if !mut_state.alpha.hide(0.3) {
                mut_state.anim_phase = AnimPhase::Hidden;

                Some(300)
            } else {
                None
            }
        }
        AnimPhase::Hidden => {
            mut_state.anim_phase = AnimPhase::Show;

            Some(60)
        }
        AnimPhase::Show => {
            if !mut_state.alpha.show(0.3) {
                mut_state.anim_phase = AnimPhase::Shown;

                Some(500)
            } else {
                None
            }
        }
        AnimPhase::NoFocus => None,
        AnimPhase::Busy => None,
    };

    let redraw_cb = mut_state.redraw_cb.upgrade().unwrap();
    let mut redraw_cb = redraw_cb.borrow_mut();
    redraw_cb.queue_redraw_cursor();

    if let Some(timeout) = next_event {
        let moved_state = state.clone();
        mut_state.timer = Some(glib::timeout_add(
            Duration::from_millis(timeout),
            move || anim_step(&moved_state),
        ));

        glib::ControlFlow::Break
    } else {
        glib::ControlFlow::Continue
    }
}

impl<CB: CursorRedrawCb> Drop for Cursor<CB> {
    fn drop(&mut self) {
        if let Some(timer_id) = self.state.borrow_mut().timer.take() {
            timer_id.remove();
        }
    }
}

pub trait CursorRedrawCb {
    fn queue_redraw_cursor(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_cursor_rect_horizontal() {
        let mut mode_data = HashMap::new();
        mode_data.insert("cursor_shape".to_owned(), From::from("horizontal"));
        mode_data.insert("cell_percentage".to_owned(), From::from(25));

        let mode_info = mode::ModeInfo::new(&mode_data).ok();
        let char_width = 50.0;
        let line_height = 30.0;
        let line_y = 0.0;

        let (y, width, height) = cursor_rect(
            mode_info.as_ref(),
            &CellMetrics::new_hw(line_height, char_width),
            line_y,
            false,
        );
        assert_eq!(line_y + line_height - line_height / 4.0, y);
        assert_eq!(char_width, width);
        assert_eq!(line_height / 4.0, height);
    }

    #[test]
    fn test_cursor_rect_horizontal_doublewidth() {
        let mut mode_data = HashMap::new();
        mode_data.insert("cursor_shape".to_owned(), From::from("horizontal"));
        mode_data.insert("cell_percentage".to_owned(), From::from(25));

        let mode_info = mode::ModeInfo::new(&mode_data).ok();
        let char_width = 50.0;
        let line_height = 30.0;
        let line_y = 0.0;

        let (y, width, height) = cursor_rect(
            mode_info.as_ref(),
            &CellMetrics::new_hw(line_height, char_width),
            line_y,
            true,
        );
        assert_eq!(line_y + line_height - line_height / 4.0, y);
        assert_eq!(char_width * 2.0, width);
        assert_eq!(line_height / 4.0, height);
    }

    #[test]
    fn test_cursor_rect_vertical() {
        let mut mode_data = HashMap::new();
        mode_data.insert("cursor_shape".to_owned(), From::from("vertical"));
        mode_data.insert("cell_percentage".to_owned(), From::from(25));

        let mode_info = mode::ModeInfo::new(&mode_data).ok();
        let char_width = 50.0;
        let line_height = 30.0;
        let line_y = 0.0;

        let (y, width, height) = cursor_rect(
            mode_info.as_ref(),
            &CellMetrics::new_hw(line_height, char_width),
            line_y,
            false,
        );
        assert_eq!(line_y, y);
        assert_eq!(char_width / 4.0, width);
        assert_eq!(line_height, height);
    }
}
