use super::*;
use gpui_component::input::{Editor, EditorState, Redo, Undo};
use rust_browser::notes::NoteRepository;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

pub(super) const NOTE_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="3" width="16" height="18" rx="2"/><path d="M8 8h8M8 12h8M8 16h5"/></svg>"#;

pub(super) struct NoteEditor {
    profile_id: String,
    profile_name: String,
    repository: NoteRepository,
    editor: Entity<EditorState>,
    generation: i64,
    saved_generation: i64,
    latest_generation: Arc<AtomicI64>,
    persisted_generation: Arc<AtomicI64>,
    persisted_revision: Arc<AtomicI64>,
    write_lock: Arc<Mutex<()>>,
    saving: bool,
    error: Option<String>,
    load_failed: bool,
    tokio: tokio::runtime::Handle,
    home: Entity<BrowserHome>,
    popover: Entity<gpui_base::PopoverState>,
    _subscription: Subscription,
}

impl NoteEditor {
    pub(super) fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.read(cx).focus_handle(cx).focus(window, cx);
    }

    pub(super) fn has_error(&self) -> bool {
        self.error.is_some()
    }

    pub(super) fn new(
        profile_id: String,
        profile_name: String,
        repository: NoteRepository,
        tokio: tokio::runtime::Handle,
        home: Entity<BrowserHome>,
        popover: Entity<gpui_base::PopoverState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (note, error) = match repository.get(&profile_id) {
            Ok(note) => (note, None),
            Err(error) => (Default::default(), Some(format!("读取失败：{error}"))),
        };
        let load_failed = error.is_some();
        let revision = note.revision;
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .line_number(false)
                .folding(false)
                .soft_wrap(true)
                .default_value(note.markdown)
        });
        let subscription = cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => this.schedule_save(cx),
                InputEvent::Blur => this.flush(cx),
                _ => {}
            },
        );
        Self {
            profile_id,
            profile_name,
            repository,
            editor,
            generation: 0,
            saved_generation: 0,
            latest_generation: Arc::new(AtomicI64::new(0)),
            persisted_generation: Arc::new(AtomicI64::new(0)),
            persisted_revision: Arc::new(AtomicI64::new(revision)),
            write_lock: Arc::new(Mutex::new(())),
            saving: false,
            error,
            load_failed,
            tokio,
            home,
            popover,
            _subscription: subscription,
        }
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if self.load_failed {
            return;
        }
        self.generation += 1;
        let generation = self.generation;
        self.latest_generation.store(generation, Ordering::SeqCst);
        let markdown = self.editor.read(cx).value().to_string();
        let repository = self.repository.clone();
        let profile_id = self.profile_id.clone();
        let latest_generation = self.latest_generation.clone();
        let persisted_generation = self.persisted_generation.clone();
        let persisted_revision = self.persisted_revision.clone();
        let write_lock = self.write_lock.clone();
        self.saving = true;
        self.error = None;
        cx.notify();
        // Tokio owns the write even if the popover is dismissed before the debounce expires.
        let write = self.tokio.spawn(async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            let _lock = write_lock.lock().expect("note write lock poisoned");
            if latest_generation.load(Ordering::SeqCst) != generation {
                return Ok::<Option<()>, anyhow::Error>(None);
            }
            let expected = persisted_revision.load(Ordering::SeqCst);
            let revision = repository.save(&profile_id, &markdown, expected)?;
            persisted_revision.store(revision, Ordering::SeqCst);
            persisted_generation.store(generation, Ordering::SeqCst);
            Ok(Some(()))
        });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation || generation <= this.saved_generation {
                    return;
                }
                match result {
                    Ok(Ok(Some(()))) => this.saved(cx),
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.failed(error.to_string(), cx),
                    Err(error) => this.failed(error.to_string(), cx),
                }
            });
        })
        .detach();
    }

    fn saved(&mut self, cx: &mut Context<Self>) {
        self.saved_generation = self.generation;
        self.saving = false;
        self.error = None;
        let has_note = !self.editor.read(cx).value().is_empty();
        let id = self.profile_id.clone();
        self.home.update(cx, |home, cx| {
            if let Some(row) = home.rows.iter_mut().find(|row| row.id == id) {
                row.note_present = has_note;
            }
            cx.notify();
        });
        cx.notify();
    }

    fn failed(&mut self, message: String, cx: &mut Context<Self>) {
        self.saving = false;
        self.error = Some(message);
        cx.notify();
    }

    pub(super) fn flush(&mut self, cx: &mut Context<Self>) {
        if self.generation <= self.saved_generation {
            return;
        }
        let write_lock = self.write_lock.clone();
        let result = {
            let _lock = write_lock.lock().expect("note write lock poisoned");
            if self.persisted_generation.load(Ordering::SeqCst) >= self.generation {
                Ok(None)
            } else {
                let markdown = self.editor.read(cx).value().to_string();
                let expected = self.persisted_revision.load(Ordering::SeqCst);
                self.repository
                    .save(&self.profile_id, &markdown, expected)
                    .map(|revision| {
                        self.persisted_revision.store(revision, Ordering::SeqCst);
                        self.persisted_generation
                            .store(self.generation, Ordering::SeqCst);
                        Some(())
                    })
            }
        };
        match result {
            Ok(_) => self.saved(cx),
            Err(error) => self.failed(error.to_string(), cx),
        }
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self.latest_generation
            .store(self.generation, Ordering::SeqCst);
        let write_lock = self.write_lock.clone();
        let result = {
            let _lock = write_lock.lock().expect("note write lock poisoned");
            self.repository.get(&self.profile_id)
        };
        match result {
            Ok(note) => {
                self.persisted_revision
                    .store(note.revision, Ordering::SeqCst);
                self.persisted_generation
                    .store(self.generation, Ordering::SeqCst);
                self.editor.update(cx, |editor, cx| {
                    editor.set_value(note.markdown, window, cx);
                });
                self.saved(cx);
            }
            Err(error) => self.failed(error.to_string(), cx),
        }
    }

    fn wrap_selection(&mut self, marker: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let selected = editor.selected_value().to_string();
            editor.replace(format!("{marker}{selected}{marker}"), window, cx);
            editor.focus_handle(cx).focus(window, cx);
        });
    }

    fn insert_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let selected = editor.selected_value().to_string();
            let text = if selected.is_empty() {
                "- ".to_string()
            } else {
                selected
                    .lines()
                    .map(|line| format!("- {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            editor.replace(text, window, cx);
            editor.focus_handle(cx).focus(window, cx);
        });
    }

    fn insert_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let selected = editor.selected_value().to_string();
            let label = if selected.is_empty() {
                "链接"
            } else {
                &selected
            };
            editor.replace(format!("[{label}](https://)"), window, cx);
            editor.focus_handle(cx).focus(window, cx);
        });
    }
}

impl Render for NoteEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = format!("备注 · {}", self.profile_name);
        let status = self.error.as_deref().unwrap_or(if self.saving {
            "保存中"
        } else {
            "已保存"
        });
        let status_color = if self.error.is_some() { RED } else { GREEN };
        let popover = self.popover.clone();
        let full_editor = self.editor.clone();
        let save_entity = cx.entity();
        let reload_entity = cx.entity();
        v_flex()
            .w(px(460.))
            .h(px(355.))
            .rounded_md()
            .border_1()
            .border_color(rgb(LINE))
            .bg(rgb(0xffffff))
            .shadow_lg()
            .overflow_hidden()
            .child(
                h_flex()
                    .h(px(34.))
                    .px_3()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .min_w_0()
                            .text_sm()
                            .font_semibold()
                            .text_ellipsis()
                            .child(title),
                    )
                    .child(
                        Button::new("note-close")
                            .ghost()
                            .xsmall()
                            .h(px(24.))
                            .w(px(24.))
                            .accessibility_label("关闭备注")
                            .label("×")
                            .on_click(move |_, window, cx| {
                                popover.update(cx, |state, cx| state.dismiss(window, cx));
                            }),
                    ),
            )
            .child(
                h_flex()
                    .h(px(32.))
                    .px_2()
                    .gap_0p5()
                    .items_center()
                    .border_t_1()
                    .border_b_1()
                    .border_color(rgb(0xf0f3f8))
                    .child(note_tool_button(
                        "note-undo",
                        "撤销",
                        "↶",
                        cx.listener(|this, _, window, cx| {
                            this.editor.read(cx).focus_handle(cx).focus(window, cx);
                            window.dispatch_action(Box::new(Undo), cx);
                        }),
                    ))
                    .child(note_tool_button(
                        "note-redo",
                        "重做",
                        "↷",
                        cx.listener(|this, _, window, cx| {
                            this.editor.read(cx).focus_handle(cx).focus(window, cx);
                            window.dispatch_action(Box::new(Redo), cx);
                        }),
                    ))
                    .child(div().mx_1().h(px(16.)).w(px(1.)).bg(rgb(LINE)))
                    .child(note_tool_button(
                        "note-bold",
                        "粗体",
                        "B",
                        cx.listener(|this, _, window, cx| {
                            this.wrap_selection("**", window, cx);
                        }),
                    ))
                    .child(note_tool_button(
                        "note-italic",
                        "斜体",
                        "I",
                        cx.listener(|this, _, window, cx| {
                            this.wrap_selection("*", window, cx);
                        }),
                    ))
                    .child(note_tool_button(
                        "note-list",
                        "列表",
                        "☷",
                        cx.listener(|this, _, window, cx| {
                            this.insert_list(window, cx);
                        }),
                    ))
                    .child(note_tool_button(
                        "note-link",
                        "链接",
                        "↗",
                        cx.listener(|this, _, window, cx| {
                            this.insert_link(window, cx);
                        }),
                    ))
                    .child(
                        gpui_base::Popover::new("note-more-popover")
                            .trigger(
                                Button::new("note-more")
                                    .ghost()
                                    .xsmall()
                                    .h(px(26.))
                                    .w(px(26.))
                                    .label("···"),
                            )
                            .content(move |_, _, _| {
                                v_flex()
                                    .w(px(150.))
                                    .p_1()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(LINE))
                                    .bg(rgb(0xffffff))
                                    .shadow_lg()
                                    .child(
                                        Button::new("note-copy-all")
                                            .ghost()
                                            .label("复制全文")
                                            .on_click({
                                                let editor = full_editor.clone();
                                                move |_, _, cx| {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(
                                                            editor.read(cx).value().to_string(),
                                                        ),
                                                    );
                                                }
                                            }),
                                    )
                                    .child(
                                        Button::new("note-save-now")
                                            .ghost()
                                            .label("立即保存")
                                            .on_click({
                                                let entity = save_entity.clone();
                                                move |_, _, cx| {
                                                    entity.update(cx, |note, cx| note.flush(cx));
                                                }
                                            }),
                                    )
                                    .child(
                                        Button::new("note-reload")
                                            .ghost()
                                            .label("重新加载")
                                            .on_click({
                                                let entity = reload_entity.clone();
                                                move |_, window, cx| {
                                                    entity.update(cx, |note, cx| {
                                                        note.reload(window, cx)
                                                    });
                                                }
                                            }),
                                    )
                            }),
                    )
                    .child(div().flex_1())
                    .child(div().size(px(6.)).rounded_full().bg(rgb(status_color)))
                    .child(
                        div()
                            .ml_1()
                            .text_xs()
                            .text_color(rgb(if self.error.is_some() { RED } else { MUTED }))
                            .child(status.to_string()),
                    ),
            )
            .child(
                div().flex_1().min_h_0().p(px(2.)).child(
                    Editor::new(&self.editor)
                        // GPUI's code editor keeps a 6px gutter and 6px text inset
                        // even with line numbers and folding disabled.
                        .ml(px(-12.))
                        .bordered(false)
                        .readonly(self.load_failed)
                        .h_full()
                        .w(px(466.))
                        .text_sm()
                        .aria_label("浏览器备注"),
                ),
            )
    }
}

fn note_tool_button(
    id: &'static str,
    tooltip: &'static str,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .ghost()
        .xsmall()
        .h(px(26.))
        .w(px(26.))
        .accessibility_label(tooltip)
        .tooltip(tooltip)
        .label(label)
        .on_click(on_click)
}
