use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, rgb, rgba, size, App, Bounds,
    ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine,
    SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use zeroize::Zeroize as _;

actions!(
    foks_text_field,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
    ]
);

pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some("FoksTextField")),
        KeyBinding::new("delete", Delete, Some("FoksTextField")),
        KeyBinding::new("left", Left, Some("FoksTextField")),
        KeyBinding::new("right", Right, Some("FoksTextField")),
        KeyBinding::new("shift-left", SelectLeft, Some("FoksTextField")),
        KeyBinding::new("shift-right", SelectRight, Some("FoksTextField")),
        KeyBinding::new("cmd-a", SelectAll, Some("FoksTextField")),
        KeyBinding::new("ctrl-a", SelectAll, Some("FoksTextField")),
        KeyBinding::new("cmd-v", Paste, Some("FoksTextField")),
        KeyBinding::new("ctrl-v", Paste, Some("FoksTextField")),
        KeyBinding::new("cmd-c", Copy, Some("FoksTextField")),
        KeyBinding::new("ctrl-c", Copy, Some("FoksTextField")),
        KeyBinding::new("cmd-x", Cut, Some("FoksTextField")),
        KeyBinding::new("ctrl-x", Cut, Some("FoksTextField")),
        KeyBinding::new("home", Home, Some("FoksTextField")),
        KeyBinding::new("end", End, Some("FoksTextField")),
    ]);
}

pub(crate) struct TextField {
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    secret: bool,
    maximum_bytes: usize,
    overflowed: bool,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    last_boundaries: Vec<(usize, usize)>,
    is_selecting: bool,
}

impl TextField {
    pub(crate) fn new(
        placeholder: impl Into<SharedString>,
        secret: bool,
        maximum_bytes: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: String::new(),
            placeholder: placeholder.into(),
            secret,
            maximum_bytes,
            overflowed: false,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            last_boundaries: vec![(0, 0)],
            is_selecting: false,
        }
    }

    pub(crate) fn value(&self) -> &str {
        &self.content
    }

    pub(crate) fn clear(&mut self, cx: &mut Context<Self>) {
        self.content.zeroize();
        self.content.clear();
        self.overflowed = false;
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout = None;
        self.last_bounds = None;
        self.last_boundaries.clear();
        self.last_boundaries.push((0, 0));
        self.is_selecting = false;
        cx.notify();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            // Trailing newlines are copy artifacts (terminals, password
            // managers), not content; substituting spaces for them would
            // silently change a secret.
            let sanitized = text
                .trim_end_matches(['\n', '\r'])
                .replace(['\n', '\r'], " ");
            self.replace_text_in_range(None, &sanitized, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.secret && !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_owned(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        self.copy(&Copy, window, cx);
        if !self.selected_range.is_empty() {
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = true;
        let offset = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .char_indices()
            .rev()
            .find_map(|(index, _)| (index < offset).then_some(index))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .char_indices()
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or(self.content.len())
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(line)) = (&self.last_bounds, &self.last_layout) else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        let display = line.closest_index_for_x(position.x - bounds.left());
        self.content_offset_for_display(display)
    }

    fn display_offset_for_content(&self, content: usize) -> usize {
        self.last_boundaries
            .iter()
            .rev()
            .find_map(|(display, original)| (*original <= content).then_some(*display))
            .unwrap_or(0)
    }

    fn content_offset_for_display(&self, display: usize) -> usize {
        self.last_boundaries
            .iter()
            .rev()
            .find_map(|(shown, original)| (*shown <= display).then_some(*original))
            .unwrap_or(0)
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for character in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += character.len_utf16();
            utf8 += character.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for character in self.content.chars() {
            if utf8 >= offset {
                break;
            }
            utf8 += character.len_utf8();
            utf16 += character.len_utf16();
        }
        utf16
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn replacement_range(&self, range_utf16: Option<&Range<usize>>) -> Range<usize> {
        range_utf16
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone())
    }

    fn replace(&mut self, range: Range<usize>, new_text: &str) {
        let mut new_text = new_text.replace(['\n', '\r'], " ");
        match compose_replacement(&self.content, &range, &new_text, self.maximum_bytes) {
            Some(replacement) => {
                if self.secret {
                    self.content.zeroize();
                }
                self.overflowed = false;
                self.content = replacement;
                let cursor = range.start + new_text.len();
                self.selected_range = cursor..cursor;
                self.selection_reversed = false;
            }
            // Rejecting the whole insertion (rather than keeping a truncated
            // prefix) means an oversized passphrase can never silently match
            // its equally-truncated confirmation.
            None => self.overflowed = true,
        }
        if self.secret {
            new_text.zeroize();
        }
    }

    fn displayed(&self) -> (SharedString, Vec<(usize, usize)>) {
        if !self.secret {
            let mut boundaries = self
                .content
                .char_indices()
                .map(|(offset, _)| (offset, offset))
                .collect::<Vec<_>>();
            boundaries.push((self.content.len(), self.content.len()));
            return (self.content.clone().into(), boundaries);
        }
        let mut output = String::new();
        let mut boundaries = Vec::new();
        for (offset, _) in self.content.char_indices() {
            boundaries.push((output.len(), offset));
            output.push('•');
        }
        boundaries.push((output.len(), self.content.len()));
        (output.into(), boundaries)
    }
}

impl Drop for TextField {
    fn drop(&mut self) {
        self.content.zeroize();
    }
}

/// Splices `new_text` into `content` over `range`, refusing the entire edit
/// when the result would exceed `maximum_bytes`. Deletions always fit.
fn compose_replacement(
    content: &str,
    range: &Range<usize>,
    new_text: &str,
    maximum_bytes: usize,
) -> Option<String> {
    let retained = content.len() - (range.end - range.start);
    if new_text.len() > maximum_bytes.saturating_sub(retained) {
        return None;
    }
    let mut replacement = String::with_capacity(retained + new_text.len());
    replacement.push_str(&content[..range.start]);
    replacement.push_str(new_text);
    replacement.push_str(&content[range.end..]);
    Some(replacement)
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        // Input services (IME reconversion, press-and-hold accents,
        // dictation) query text through this path; a secret field must never
        // hand its contents to them.
        if self.secret {
            return None;
        }
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.replacement_range(range_utf16.as_ref());
        self.replace(range, new_text);
        self.marked_range = None;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.replacement_range(range_utf16.as_ref());
        let start = range.start;
        self.replace(range, new_text);
        let end = self.selected_range.end;
        self.marked_range = (end > start).then_some(start..end);
        if let Some(selected) = new_selected_range_utf16 {
            let marked = &self.content[start..end];
            let selected_start = utf8_offset_from_utf16(marked, selected.start);
            let selected_end = utf8_offset_from_utf16(marked, selected.end);
            self.selected_range = start + selected_start..start + selected_end.min(marked.len());
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let start = self.display_offset_for_content(range.start);
        let end = self.display_offset_for_content(range.end);
        Some(Bounds::from_corners(
            point(bounds.left() + layout.x_for_index(start), bounds.top()),
            point(bounds.left() + layout.x_for_index(end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let layout = self.last_layout.as_ref()?;
        bounds.localize(&point)?;
        let display = layout.index_for_x(point.x - bounds.left())?;
        Some(self.offset_to_utf16(self.content_offset_for_display(display)))
    }
}

fn utf8_offset_from_utf16(value: &str, offset: usize) -> usize {
    let mut utf8 = 0;
    let mut utf16 = 0;
    for character in value.chars() {
        if utf16 >= offset {
            break;
        }
        utf16 += character.len_utf16();
        utf8 += character.len_utf8();
    }
    utf8
}

struct TextElement {
    input: Entity<TextField>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    boundaries: Vec<(usize, usize)>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let (display, boundaries) = input.displayed();
        let empty = display.is_empty();
        let shown = if empty {
            input.placeholder.clone()
        } else {
            display
        };
        let run = TextRun {
            len: shown.len(),
            font: window.text_style().font(),
            color: if empty {
                rgba(0x65738a80).into()
            } else {
                window.text_style().color
            },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked) = &input.marked_range {
            let start = boundaries
                .iter()
                .rev()
                .find_map(|(display, source)| (*source <= marked.start).then_some(*display))
                .unwrap_or(0);
            let end = boundaries
                .iter()
                .rev()
                .find_map(|(display, source)| (*source <= marked.end).then_some(*display))
                .unwrap_or(start);
            [
                TextRun {
                    len: start,
                    ..run.clone()
                },
                TextRun {
                    len: end.saturating_sub(start),
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: shown.len().saturating_sub(end),
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };
        let line = window.text_system().shape_line(
            shown,
            window.text_style().font_size.to_pixels(window.rem_size()),
            &runs,
            None,
        );
        let display_offset = |source| {
            boundaries
                .iter()
                .rev()
                .find_map(|(display, original)| (*original <= source).then_some(*display))
                .unwrap_or(0)
        };
        let selected =
            display_offset(input.selected_range.start)..display_offset(input.selected_range.end);
        let cursor = display_offset(input.cursor_offset());
        let (selection, cursor) = if selected.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + line.x_for_index(cursor), bounds.top()),
                        size(px(2.), bounds.bottom() - bounds.top()),
                    ),
                    rgb(0x2764d8),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(selected.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(selected.end),
                            bounds.bottom(),
                        ),
                    ),
                    rgba(0x2764d833),
                )),
                None,
            )
        };
        PrepaintState {
            line: Some(line),
            cursor,
            selection,
            boundaries,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = state.selection.take() {
            window.paint_quad(selection);
        }
        let line = state
            .line
            .take()
            .expect("text field prepaint creates a line");
        line.paint(bounds.origin, window.line_height(), window, cx)
            .expect("text field line paint succeeds");
        if focus.is_focused(window) {
            if let Some(cursor) = state.cursor.take() {
                window.paint_quad(cursor);
            }
        }
        self.input.update(cx, |input, _| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
            input.last_boundaries = std::mem::take(&mut state.boundaries);
        });
    }
}

impl Render for TextField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("FoksTextField")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .h(px(38.))
            .w_full()
            .px_2()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(if self.overflowed {
                rgb(0xc0392b)
            } else {
                rgb(0xb8c3d3)
            })
            .bg(rgb(0xffffff))
            .child(TextElement { input: cx.entity() })
    }
}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::compose_replacement;

    #[test]
    fn oversized_insertions_are_rejected_whole_rather_than_truncated() {
        assert_eq!(
            compose_replacement("abc", &(3..3), "de", 5),
            Some("abcde".to_owned())
        );
        assert_eq!(compose_replacement("abc", &(3..3), "def", 5), None);
        assert_eq!(compose_replacement("", &(0..0), "too long", 7), None);
        // A replacement that frees enough room still fits.
        assert_eq!(
            compose_replacement("abcde", &(0..3), "xy", 5),
            Some("xyde".to_owned())
        );
        // Deletions always fit, even at the cap.
        assert_eq!(
            compose_replacement("abcde", &(4..5), "", 5),
            Some("abcd".to_owned())
        );
    }
}
