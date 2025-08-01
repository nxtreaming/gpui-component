use std::{ops::Range, rc::Rc};

use gpui::{
    fill, point, px, relative, size, App, Bounds, Corners, Element, ElementId, ElementInputHandler,
    Entity, GlobalElementId, HighlightStyle, IntoElement, LayoutId, MouseButton, MouseMoveEvent,
    Path, Pixels, Point, SharedString, Size, Style, TextAlign, TextRun, UnderlineStyle, Window,
    WrappedLine,
};
use smallvec::SmallVec;

use crate::{
    highlighter::SyntaxHighlighter, input::blink_cursor::CURSOR_WIDTH, ActiveTheme as _, Root,
};

use super::{mode::InputMode, InputState, LastLayout};

const RIGHT_MARGIN: Pixels = px(5.);
const BOTTOM_MARGIN_ROWS: usize = 1;
const LINE_NUMBER_MARGIN_RIGHT: Pixels = px(10.);

pub(super) struct TextElement {
    state: Entity<InputState>,
    placeholder: SharedString,
}

impl TextElement {
    pub(super) fn new(state: Entity<InputState>) -> Self {
        Self {
            state,
            placeholder: SharedString::default(),
        }
    }

    /// Set the placeholder text of the input field.
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    fn paint_mouse_listeners(&mut self, window: &mut Window, _: &mut App) {
        window.on_mouse_event({
            let state = self.state.clone();

            move |event: &MouseMoveEvent, _, window, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    state.update(cx, |state, cx| {
                        state.on_drag_move(event, window, cx);
                    });
                }
            }
        });
    }

    /// Returns the:
    ///
    /// - cursor bounds
    /// - scroll offset
    /// - current line index
    fn layout_cursor(
        &self,
        lines: &[WrappedLine],
        line_height: Pixels,
        bounds: &mut Bounds<Pixels>,
        line_number_width: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) -> (Option<Bounds<Pixels>>, Point<Pixels>, Option<usize>) {
        let state = self.state.read(cx);
        let mut selected_range = state.selected_range;
        if let Some(marked_range) = &state.marked_range {
            selected_range = (marked_range.end..marked_range.end).into();
        }

        let cursor = state.cursor();
        let mut current_line_index = None;
        let mut scroll_offset = state.scroll_handle.offset();
        let mut cursor_bounds = None;

        // If the input has a fixed height (Otherwise is auto-grow), we need to add a bottom margin to the input.
        let bottom_margin = if state.mode.is_auto_grow() {
            px(0.) + line_height
        } else {
            BOTTOM_MARGIN_ROWS * line_height + line_height
        };
        // The cursor corresponds to the current cursor position in the text no only the line.
        let mut cursor_pos = None;
        let mut cursor_start = None;
        let mut cursor_end = None;

        let mut prev_lines_offset = 0;
        let mut offset_y = px(0.);
        for (line_ix, line) in lines.iter().enumerate() {
            // break loop if all cursor positions are found
            if cursor_pos.is_some() && cursor_start.is_some() && cursor_end.is_some() {
                break;
            }

            let line_origin = point(px(0.), offset_y);
            if cursor_pos.is_none() {
                let offset = cursor.offset.saturating_sub(prev_lines_offset);

                if let Some(pos) = line.position_for_index(offset, line_height) {
                    current_line_index = Some(line_ix);
                    cursor_pos = Some(line_origin + pos);
                }
            }
            if cursor_start.is_none() {
                let offset = selected_range.start.saturating_sub(prev_lines_offset);
                if let Some(pos) = line.position_for_index(offset, line_height) {
                    cursor_start = Some(line_origin + pos);
                }
            }
            if cursor_end.is_none() {
                let offset = selected_range.end.saturating_sub(prev_lines_offset);
                if let Some(pos) = line.position_for_index(offset, line_height) {
                    cursor_end = Some(line_origin + pos);
                }
            }

            offset_y += line.size(line_height).height;
            // +1 for skip the last `\n`
            prev_lines_offset += line.len() + 1;
        }

        if let (Some(cursor_pos), Some(cursor_start), Some(cursor_end)) =
            (cursor_pos, cursor_start, cursor_end)
        {
            let cursor_moved = state.last_cursor != Some(cursor);
            let selection_changed = state.last_selected_range != Some(selected_range);

            if cursor_moved || selection_changed {
                scroll_offset.x =
                    if scroll_offset.x + cursor_pos.x > (bounds.size.width - RIGHT_MARGIN) {
                        // cursor is out of right
                        bounds.size.width - RIGHT_MARGIN - cursor_pos.x
                    } else if scroll_offset.x + cursor_pos.x < px(0.) {
                        // cursor is out of left
                        scroll_offset.x - cursor_pos.x
                    } else {
                        scroll_offset.x
                    };
                scroll_offset.y = if scroll_offset.y + cursor_pos.y + line_height
                    > bounds.size.height - bottom_margin
                {
                    // cursor is out of bottom
                    bounds.size.height - bottom_margin - cursor_pos.y
                } else if scroll_offset.y + cursor_pos.y < px(0.) {
                    // cursor is out of top
                    scroll_offset.y - cursor_pos.y
                } else {
                    scroll_offset.y
                };

                if state.selection_reversed {
                    if scroll_offset.x + cursor_start.x < px(0.) {
                        // selection start is out of left
                        scroll_offset.x = -cursor_start.x;
                    }
                    if scroll_offset.y + cursor_start.y < px(0.) {
                        // selection start is out of top
                        scroll_offset.y = -cursor_start.y;
                    }
                } else {
                    if scroll_offset.x + cursor_end.x <= px(0.) {
                        // selection end is out of left
                        scroll_offset.x = -cursor_end.x;
                    }
                    if scroll_offset.y + cursor_end.y <= px(0.) {
                        // selection end is out of top
                        scroll_offset.y = -cursor_end.y;
                    }
                }
            }

            if state.show_cursor(window, cx) {
                // cursor blink
                let cursor_height = line_height;
                cursor_bounds = Some(Bounds::new(
                    point(
                        bounds.left() + cursor_pos.x + line_number_width + scroll_offset.x,
                        bounds.top() + cursor_pos.y + ((line_height - cursor_height) / 2.),
                    ),
                    size(CURSOR_WIDTH, cursor_height),
                ));
            };
        }

        bounds.origin = bounds.origin + scroll_offset;

        (cursor_bounds, scroll_offset, current_line_index)
    }

    fn layout_selections(
        &self,
        lines: &[WrappedLine],
        line_height: Pixels,
        bounds: &mut Bounds<Pixels>,
        line_number_width: Pixels,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<Path<Pixels>> {
        let state = self.state.read(cx);
        let mut selected_range = state.selected_range;
        if let Some(marked_range) = &state.marked_range {
            if !marked_range.is_empty() {
                selected_range = (marked_range.end..marked_range.end).into();
            }
        }
        if selected_range.is_empty() {
            return None;
        }

        let (start_ix, end_ix) = if selected_range.start < selected_range.end {
            (selected_range.start, selected_range.end)
        } else {
            (selected_range.end, selected_range.start)
        };

        let mut prev_lines_offset = 0;
        let mut line_corners = vec![];

        let mut offset_y = px(0.);
        for line in lines.iter() {
            let line_size = line.size(line_height);
            let line_wrap_width = line_size.width;

            let line_origin = point(px(0.), offset_y);

            let line_cursor_start =
                line.position_for_index(start_ix.saturating_sub(prev_lines_offset), line_height);
            let line_cursor_end =
                line.position_for_index(end_ix.saturating_sub(prev_lines_offset), line_height);

            if line_cursor_start.is_some() || line_cursor_end.is_some() {
                let start = line_cursor_start
                    .unwrap_or_else(|| line.position_for_index(0, line_height).unwrap());

                let end = line_cursor_end
                    .unwrap_or_else(|| line.position_for_index(line.len(), line_height).unwrap());

                // Split the selection into multiple items
                let wrapped_lines =
                    (end.y / line_height).ceil() as usize - (start.y / line_height).ceil() as usize;

                let mut end_x = end.x;
                if wrapped_lines > 0 {
                    end_x = line_wrap_width;
                }

                // Ensure at least 6px width for the selection for empty lines.
                end_x = end_x.max(start.x + px(6.));

                line_corners.push(Corners {
                    top_left: line_origin + point(start.x, start.y),
                    top_right: line_origin + point(end_x, start.y),
                    bottom_left: line_origin + point(start.x, start.y + line_height),
                    bottom_right: line_origin + point(end_x, start.y + line_height),
                });

                // wrapped lines
                for i in 1..=wrapped_lines {
                    let start = point(px(0.), start.y + i as f32 * line_height);
                    let mut end = point(end.x, end.y + i as f32 * line_height);
                    if i < wrapped_lines {
                        end.x = line_size.width;
                    }

                    line_corners.push(Corners {
                        top_left: line_origin + point(start.x, start.y),
                        top_right: line_origin + point(end.x, start.y),
                        bottom_left: line_origin + point(start.x, start.y + line_height),
                        bottom_right: line_origin + point(end.x, start.y + line_height),
                    });
                }
            }

            if line_cursor_start.is_some() && line_cursor_end.is_some() {
                break;
            }

            offset_y += line_size.height;
            // +1 for skip the last `\n`
            prev_lines_offset += line.len() + 1;
        }

        let mut points = vec![];
        if line_corners.is_empty() {
            return None;
        }

        // Fix corners to make sure the left to right direction
        for corners in &mut line_corners {
            if corners.top_left.x > corners.top_right.x {
                std::mem::swap(&mut corners.top_left, &mut corners.top_right);
                std::mem::swap(&mut corners.bottom_left, &mut corners.bottom_right);
            }
        }

        for corners in &line_corners {
            points.push(corners.top_right);
            points.push(corners.bottom_right);
            points.push(corners.bottom_left);
        }

        let mut rev_line_corners = line_corners.iter().rev().peekable();
        while let Some(corners) = rev_line_corners.next() {
            points.push(corners.top_left);
            if let Some(next) = rev_line_corners.peek() {
                if next.top_left.x > corners.top_left.x {
                    points.push(point(next.top_left.x, corners.top_left.y));
                }
            }
        }

        // print_points_as_svg_path(&line_corners, &points);

        let path_origin = bounds.origin + point(line_number_width, px(0.));
        let first_p = *points.get(0).unwrap();
        let mut builder = gpui::PathBuilder::fill();
        builder.move_to(path_origin + first_p);
        for p in points.iter().skip(1) {
            builder.line_to(path_origin + *p);
        }

        builder.build().ok()
    }

    /// Calculate the visible range of lines in the viewport.
    ///
    /// The visible range is based on unwrapped lines (Zero based).
    fn calculate_visible_range(
        &self,
        state: &InputState,
        line_height: Pixels,
        input_height: Pixels,
    ) -> Range<usize> {
        if state.mode.is_single_line() {
            return 0..1;
        }

        let scroll_top = -state.scroll_handle.offset().y;
        let total_lines = state.text_wrapper.lines.len();
        let mut visible_range = 0..total_lines;
        let mut line_top = px(0.);
        for (ix, line) in state.text_wrapper.lines.iter().enumerate() {
            line_top += line.height(line_height);

            if line_top < scroll_top {
                visible_range.start = ix;
            }

            if line_top > scroll_top + input_height {
                visible_range.end = (ix + 1).min(total_lines);
                break;
            }
        }

        visible_range
    }

    /// First usize is the offset of skipped.
    fn highlight_lines(
        &mut self,
        visible_range: &Range<usize>,
        cx: &mut App,
    ) -> Option<(usize, Vec<(Range<usize>, HighlightStyle)>)> {
        let theme = cx.theme().highlight_theme.clone();
        self.state.update(cx, |state, cx| match &state.mode {
            InputMode::CodeEditor {
                language,
                highlighter,
                markers,
                ..
            } => {
                // Init highlighter if not initialized
                let mut highlighter = highlighter.borrow_mut();
                if highlighter.is_none() {
                    highlighter.replace(SyntaxHighlighter::new(language, cx));
                };
                let Some(highlighter) = highlighter.as_ref() else {
                    return None;
                };

                let mut offset = 0;
                let mut skipped_offset = 0;
                let mut styles = vec![];

                for (ix, line) in state.text.split('\n').enumerate() {
                    // +1 for last `\n`.
                    let line_len = line.len() + 1;
                    if ix < visible_range.start {
                        offset += line_len;
                        skipped_offset = offset;
                        continue;
                    }
                    if ix > visible_range.end {
                        break;
                    }

                    let range = offset..offset + line_len;
                    let line_styles = highlighter.styles(&range, &theme);
                    styles = gpui::combine_highlights(styles, line_styles).collect();

                    offset = range.end;
                }

                let mut marker_styles = vec![];
                for marker in markers.iter() {
                    if let Some(range) = &marker.range {
                        if range.start < skipped_offset {
                            continue;
                        }

                        let node_range = range.start..range.end;
                        if node_range.start >= visible_range.start
                            || node_range.end <= visible_range.end
                        {
                            marker_styles
                                .push((node_range, marker.severity.highlight_style(&theme, cx)));
                        }
                    }
                }

                styles = gpui::combine_highlights(marker_styles, styles).collect();

                Some((skipped_offset, styles))
            }
            _ => None,
        })
    }
}

pub(super) struct PrepaintState {
    /// The lines of entire lines.
    last_layout: LastLayout,
    /// The lines only contains the visible lines in the viewport, based on `visible_range`.
    line_numbers: Option<Vec<SmallVec<[WrappedLine; 1]>>>,
    line_number_width: Pixels,
    /// Size of the scrollable area by entire lines.
    scroll_size: Size<Pixels>,
    cursor_bounds: Option<Bounds<Pixels>>,
    cursor_scroll_offset: Point<Pixels>,
    /// line index (zero based), no wrap, same line as the cursor.
    current_line_index: Option<usize>,
    selection_path: Option<Path<Pixels>>,
    bounds: Bounds<Pixels>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// A debug function to print points as SVG path.
#[allow(unused)]
fn print_points_as_svg_path(
    line_corners: &Vec<Corners<Point<Pixels>>>,
    points: &Vec<Point<Pixels>>,
) {
    for corners in line_corners {
        println!(
            "tl: ({}, {}), tr: ({}, {}), bl: ({}, {}), br: ({}, {})",
            corners.top_left.x.0 as i32,
            corners.top_left.y.0 as i32,
            corners.top_right.x.0 as i32,
            corners.top_right.y.0 as i32,
            corners.bottom_left.x.0 as i32,
            corners.bottom_left.y.0 as i32,
            corners.bottom_right.x.0 as i32,
            corners.bottom_right.y.0 as i32,
        );
    }

    if points.len() > 0 {
        println!("M{},{}", points[0].x.0 as i32, points[0].y.0 as i32);
        for p in points.iter().skip(1) {
            println!("L{},{}", p.x.0 as i32, p.y.0 as i32);
        }
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let state = self.state.read(cx);
        let line_height = window.line_height();

        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if state.mode.is_multi_line() {
            style.flex_grow = 1.0;
            if let Some(h) = state.mode.height() {
                style.size.height = h.into();
                style.min_size.height = line_height.into();
            } else {
                style.size.height = relative(1.).into();
                style.min_size.height = (state.mode.rows() * line_height).into();
            }
        } else {
            // For single-line inputs, the minimum height should be the line height
            style.size.height = line_height.into();
        };

        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let state = self.state.read(cx);
        let line_height = window.line_height();

        let visible_range = self.calculate_visible_range(&state, line_height, bounds.size.height);
        let highlight_styles = self.highlight_lines(&visible_range, cx);

        let state = self.state.read(cx);
        let multi_line = state.mode.is_multi_line();
        let text = state.text.clone();
        let is_empty = text.is_empty();
        let placeholder = self.placeholder.clone();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let mut bounds = bounds;

        let (display_text, text_color) = if is_empty {
            (placeholder, cx.theme().muted_foreground)
        } else if state.masked {
            (
                "*".repeat(text.chars().count()).into(),
                cx.theme().foreground,
            )
        } else {
            (text.clone(), cx.theme().foreground)
        };

        let text_style = window.text_style();

        // Calculate the width of the line numbers
        let empty_line_number = window
            .text_system()
            .shape_text(
                "++++".into(),
                font_size,
                &[TextRun {
                    len: 4,
                    font: style.font(),
                    color: gpui::black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
                None,
            )
            .unwrap();
        let line_number_width = if state.mode.line_number() {
            empty_line_number.last().unwrap().width() + LINE_NUMBER_MARGIN_RIGHT
        } else {
            px(0.)
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let marked_run = TextRun {
            len: 0,
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(text_color),
                wavy: false,
            }),
            strikethrough: None,
        };

        let runs = if !is_empty {
            if let Some((skipped_offset, highlight_styles)) = highlight_styles {
                let mut runs = vec![];
                if skipped_offset > 0 {
                    runs.push(TextRun {
                        len: skipped_offset,
                        ..run.clone()
                    });
                }

                runs.extend(highlight_styles.iter().map(|(range, style)| {
                    let mut run = text_style.clone().highlight(*style).to_run(range.len());
                    if let Some(marked_range) = &state.marked_range {
                        if range.start >= marked_range.start && range.end <= marked_range.end {
                            run.color = marked_run.color;
                            run.strikethrough = marked_run.strikethrough;
                            run.underline = marked_run.underline;
                        }
                    }

                    run
                }));

                runs.into_iter().filter(|run| run.len > 0).collect()
            } else {
                vec![run]
            }
        } else if let Some(marked_range) = &state.marked_range {
            // IME marked text
            vec![
                TextRun {
                    len: marked_range.start.offset,
                    ..run.clone()
                },
                TextRun {
                    len: marked_range.end.offset - marked_range.start.offset,
                    underline: marked_run.underline,
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - marked_range.end.offset,
                    ..run.clone()
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let wrap_width = if multi_line {
            Some(bounds.size.width - line_number_width - RIGHT_MARGIN)
        } else {
            None
        };

        // NOTE: If there have about 10K lines, this will take about 5~6ms.
        // let measure = Measure::new("shape_text");
        let lines = window
            .text_system()
            .shape_text(display_text, font_size, &runs, wrap_width, None)
            .expect("failed to shape text");
        // measure.end();

        let total_wrapped_lines = lines
            .iter()
            .map(|line| {
                // +1 is the first line, `wrap_boundaries` is the wrapped lines after the `\n`.
                1 + line.wrap_boundaries.len()
            })
            .sum::<usize>();

        let max_line_width = lines
            .iter()
            .map(|line| line.width())
            .max()
            .unwrap_or(bounds.size.width);
        let scroll_size = size(
            max_line_width + line_number_width + RIGHT_MARGIN,
            (total_wrapped_lines as f32 * line_height).max(bounds.size.height),
        );

        // `position_for_index` for example
        //
        // #### text
        //
        // Hello 世界，this is GPUI component.
        // The GPUI Component is a collection of UI components for
        // GPUI framework, including Button, Input, Checkbox, Radio,
        // Dropdown, Tab, and more...
        //
        // wrap_width: 444px, line_height: 20px
        //
        // #### lines[0]
        //
        // | index | pos              | line |
        // |-------|------------------|------|
        // | 5     | (37 px, 0.0)     | 0    |
        // | 38    | (261.7 px, 20.0) | 0    |
        // | 40    | None             | -    |
        //
        // #### lines[1]
        //
        // | index | position              | line |
        // |-------|-----------------------|------|
        // | 5     | (43.578125 px, 0.0)   | 0    |
        // | 56    | (422.21094 px, 0.0)   | 0    |
        // | 57    | (11.6328125 px, 20.0) | 1    |
        // | 114   | (429.85938 px, 20.0)  | 1    |
        // | 115   | (11.3125 px, 40.0)    | 2    |

        // Calculate the scroll offset to keep the cursor in view

        let (cursor_bounds, cursor_scroll_offset, current_line_index) = self.layout_cursor(
            &lines,
            line_height,
            &mut bounds,
            line_number_width,
            window,
            cx,
        );

        let selection_path = self.layout_selections(
            &lines,
            line_height,
            &mut bounds,
            line_number_width,
            window,
            cx,
        );

        let state = self.state.read(cx);
        let line_numbers = if state.mode.line_number() {
            let mut line_numbers = vec![];
            let run_len = 4;
            let other_line_runs = vec![TextRun {
                len: run_len,
                font: style.font(),
                color: cx.theme().muted_foreground,
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let current_line_runs = vec![TextRun {
                len: run_len,
                font: style.font(),
                color: cx.theme().foreground,
                background_color: None,
                underline: None,
                strikethrough: None,
            }];

            // build line numbers
            for (ix, line) in lines
                .iter()
                .skip(visible_range.start)
                .take(visible_range.len())
                .enumerate()
            {
                let ix = ix + visible_range.start;
                let line_no = ix + 1;

                let mut line_no_text = format!("{:>4}", line_no);
                if !line.wrap_boundaries.is_empty() {
                    line_no_text.push_str(&"\n    ".repeat(line.wrap_boundaries.len()));
                }

                let runs = if current_line_index == Some(ix) {
                    &current_line_runs
                } else {
                    &other_line_runs
                };

                let shape_line = window
                    .text_system()
                    .shape_text(line_no_text.into(), font_size, &runs, None, None)
                    .unwrap();
                line_numbers.push(shape_line);
            }
            Some(line_numbers)
        } else {
            None
        };

        PrepaintState {
            bounds,
            last_layout: LastLayout {
                lines: Rc::new(lines),
                line_height,
                visible_range,
            },
            scroll_size,
            line_numbers,
            line_number_width,
            cursor_bounds,
            cursor_scroll_offset,
            current_line_index,
            selection_path,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        input_bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.state.read(cx).focus_handle.clone();
        let focused = focus_handle.is_focused(window);
        let bounds = prepaint.bounds;
        let selected_range = self.state.read(cx).selected_range;
        let visible_range = &prepaint.last_layout.visible_range;

        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.state.clone()),
            cx,
        );

        // Set Root focused_input when self is focused
        if focused {
            let state = self.state.clone();
            if Root::read(window, cx).focused_input.as_ref() != Some(&state) {
                Root::update(window, cx, |root, _, cx| {
                    root.focused_input = Some(state);
                    cx.notify();
                });
            }
        }

        // And reset focused_input when next_frame start
        window.on_next_frame({
            let state = self.state.clone();
            move |window, cx| {
                if !focused && Root::read(window, cx).focused_input.as_ref() == Some(&state) {
                    Root::update(window, cx, |root, _, cx| {
                        root.focused_input = None;
                        cx.notify();
                    });
                }
            }
        });

        // Paint multi line text
        let line_height = window.line_height();
        let origin = bounds.origin;

        let mut invisible_top_padding = px(0.);
        for line in prepaint.last_layout.lines.iter().take(visible_range.start) {
            invisible_top_padding += line.size(line_height).height;
        }

        let mut mask_offset_y = px(0.);
        if self.state.read(cx).masked {
            // Move down offset for vertical centering the *****
            if cfg!(target_os = "macos") {
                mask_offset_y = px(3.);
            } else {
                mask_offset_y = px(2.5);
            }
        }

        let active_line_color = cx.theme().highlight_theme.style.active_line;

        let mut offset_y = px(0.);
        if let Some(line_numbers) = prepaint.line_numbers.as_ref() {
            offset_y += invisible_top_padding;

            // Each item is the normal lines.
            for (ix, lines) in line_numbers.iter().enumerate() {
                let is_active = prepaint.current_line_index == Some(visible_range.start + ix);
                for line in lines {
                    let p = point(origin.x, origin.y + offset_y);
                    let line_size = line.size(line_height);
                    // Paint the current line background
                    if is_active {
                        if let Some(bg_color) = active_line_color {
                            window.paint_quad(fill(
                                Bounds::new(p, size(bounds.size.width, line_height)),
                                bg_color,
                            ));
                        }
                    }
                    _ = line.paint(p, line_height, TextAlign::Left, None, window, cx);
                    offset_y += line_size.height;
                }
            }
        }

        // Paint selections
        if window.is_window_active() {
            if let Some(path) = prepaint.selection_path.take() {
                window.paint_path(path, cx.theme().selection);
            }
        }

        // Paint text
        let mut offset_y = mask_offset_y + invisible_top_padding;

        for line in prepaint
            .last_layout
            .iter()
            .skip(visible_range.start)
            .take(visible_range.len())
        {
            let p = point(origin.x + prepaint.line_number_width, origin.y + offset_y);
            _ = line.paint(p, line_height, TextAlign::Left, None, window, cx);
            offset_y += line.size(line_height).height;
        }

        if focused {
            if let Some(mut cursor_bounds) = prepaint.cursor_bounds.take() {
                cursor_bounds.origin.y += prepaint.cursor_scroll_offset.y;
                window.paint_quad(fill(cursor_bounds, cx.theme().caret));
            }
        }

        self.state.update(cx, |state, cx| {
            state.last_layout = Some(prepaint.last_layout.clone());
            state.last_bounds = Some(bounds);
            state.last_cursor = Some(state.cursor());
            state.set_input_bounds(input_bounds, cx);
            state.last_selected_range = Some(selected_range);
            state.scroll_size = prepaint.scroll_size;
            state.line_number_width = prepaint.line_number_width;
            state
                .scroll_handle
                .set_offset(prepaint.cursor_scroll_offset);
            cx.notify();
        });

        self.paint_mouse_listeners(window, cx);
    }
}
