//! Headless tests: real egui frames with synthetic input, so caret, selection, undo and speed
//! are checked through the same path the window uses.

use std::time::Instant;

use egui::{Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};

use super::SourceEditor;
use crate::theme;

struct Rig {
    ctx: Context,
    editor: SourceEditor,
    text: String,
    time: f64,
    markdown: bool,
}

impl Rig {
    fn new(text: &str) -> Self {
        let ctx = Context::default();
        Self { ctx, editor: SourceEditor::default(), text: text.to_owned(), time: 0.0, markdown: true }
    }

    fn frame(&mut self, events: Vec<Event>) -> super::EditorResponse {
        self.time += 0.016;
        let mut raw = RawInput::default();
        raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        raw.time = Some(self.time);
        raw.events = events;
        let mut response = None;
        let _ = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                response = Some(self.editor.show(
                    ui,
                    "test",
                    &mut self.text,
                    self.markdown,
                    false,
                    theme::palette(false),
                    0,
                    0.0,
                ));
            });
        });
        response.unwrap()
    }

    /// Clicks into the text area so the editor owns the keyboard.
    fn focus(&mut self) {
        self.frame(vec![]);
        let at = Pos2::new(300.0, 14.0);
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        self.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) -> super::EditorResponse {
        self.frame(vec![Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }])
    }

    fn type_text(&mut self, text: &str) -> super::EditorResponse {
        self.frame(vec![Event::Text(text.to_owned())])
    }
}

const SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::NONE };
const CTRL: Modifiers = Modifiers { command: true, ctrl: true, ..Modifiers::NONE };

#[test]
fn click_types_and_undoes() {
    let mut rig = Rig::new("hello\nworld\n");
    rig.focus();
    // The click landed on the first line, far right of the text: the caret sits at its end.
    let response = rig.type_text("!");
    assert!(response.changed);
    assert_eq!(rig.text, "hello!\nworld\n");
    rig.type_text("?");
    assert_eq!(rig.text, "hello!?\nworld\n");
    rig.key(Key::Z, CTRL);
    assert_eq!(rig.text, "hello\nworld\n", "typing in one burst is one undo step");
    rig.key(Key::Y, CTRL);
    assert_eq!(rig.text, "hello!?\nworld\n");
}

#[test]
fn selection_navigation_and_delete() {
    let mut rig = Rig::new("one two\nthree\n");
    rig.focus();
    rig.key(Key::Home, Modifiers::NONE);
    rig.key(Key::ArrowRight, CTRL | SHIFT);
    rig.key(Key::Backspace, Modifiers::NONE);
    assert_eq!(rig.text, "two\nthree\n");
    rig.key(Key::ArrowDown, Modifiers::NONE);
    rig.key(Key::End, SHIFT);
    let response = rig.key(Key::ArrowLeft, SHIFT);
    assert_eq!(response.cursor, 8, "shift+left shrinks the selection by one");
    rig.key(Key::A, CTRL);
    rig.type_text("x");
    assert_eq!(rig.text, "x");
}

#[test]
fn enter_keeps_indent_and_continues_lists() {
    let mut rig = Rig::new("  - item");
    rig.focus();
    rig.key(Key::End, Modifiers::NONE);
    rig.key(Key::Enter, Modifiers::NONE);
    assert_eq!(rig.text, "  - item\n  - ");
    rig.key(Key::Enter, Modifiers::NONE);
    assert_eq!(rig.text, "  - item\n", "an empty item ends the list");
}

#[test]
fn tab_indents_selected_lines() {
    let mut rig = Rig::new("a\nb\nc");
    rig.focus();
    rig.key(Key::A, CTRL);
    rig.key(Key::Tab, Modifiers::NONE);
    assert_eq!(rig.text, "    a\n    b\n    c");
    rig.key(Key::Tab, SHIFT);
    assert_eq!(rig.text, "a\nb\nc");
}

#[test]
fn move_lines_and_find() {
    let mut rig = Rig::new("a\nb\nc");
    rig.focus();
    rig.key(Key::Home, CTRL);
    rig.key(Key::ArrowDown, Modifiers::ALT);
    assert_eq!(rig.text, "b\na\nc");
    rig.editor.find = "c".into();
    rig.editor.find_open = true;
    rig.editor.find_step(&rig.text.clone(), true);
    assert_eq!(rig.editor.sel.range(), (4, 5));
}

/// Frame time at 400k lines must not depend on the size of the file.
#[test]
fn big_file_frames_stay_fast() {
    let mut text = String::new();
    for i in 0..400_000 {
        text.push_str(&format!("    node{i}[Label {i}] --> node{}[Next]\n", i + 1));
    }
    let mut rig = Rig::new(&text);
    rig.markdown = false;
    rig.focus();
    let mut measure = |rig: &mut Rig, name: &str, events: &dyn Fn() -> Vec<Event>, n: usize| -> f64 {
        let started = Instant::now();
        for _ in 0..n {
            rig.frame(events());
        }
        let ms = started.elapsed().as_secs_f64() * 1000.0 / n as f64;
        eprintln!("{name}: {ms:.2} ms/frame");
        ms
    };
    let key = |key: Key, modifiers: Modifiers| {
        move || vec![Event::Key { key, physical_key: Some(key), pressed: true, repeat: true, modifiers }]
    };
    let idle = measure(&mut rig, "idle", &Vec::new, 30);
    let down = measure(&mut rig, "arrow down", &key(Key::ArrowDown, Modifiers::NONE), 60);
    let select = measure(&mut rig, "shift+arrow down", &key(Key::ArrowDown, SHIFT), 60);
    let word = measure(&mut rig, "ctrl+shift+right", &key(Key::ArrowRight, CTRL | SHIFT), 60);
    let page = measure(&mut rig, "page down", &key(Key::PageDown, Modifiers::NONE), 60);
    let typed = measure(&mut rig, "typing", &|| vec![Event::Text("a".into())], 60);
    rig.key(Key::End, CTRL);
    let end = measure(&mut rig, "after ctrl+end", &Vec::new, 10);
    for (name, ms) in [("idle", idle), ("down", down), ("select", select), ("word", word), ("page", page), ("typing", typed), ("end", end)] {
        assert!(ms < 12.0, "{name} took {ms:.2} ms per frame on a 400k-line file");
    }
}
