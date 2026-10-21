use super::*;

#[derive(Clone)]
struct StartupTabOption {
    preset: BrowserTabPreset,
}

impl SelectItem for StartupTabOption {
    type Value = BrowserTabPreset;

    fn title(&self) -> SharedString {
        self.preset.name.clone().into()
    }
    fn value(&self) -> &Self::Value {
        &self.preset
    }
    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.preset.name.to_lowercase().contains(&query)
            || self.preset.url.to_lowercase().contains(&query)
    }
}

pub(super) struct StartupTabField {
    pub(super) input: Entity<InputState>,
    select: Entity<SelectState<Vec<StartupTabOption>>>,
    _subscriptions: Vec<Subscription>,
}

impl StartupTabField {
    pub(super) fn new(
        value: &str,
        presets: &[BrowserTabPreset],
        window: &mut Window,
        cx: &mut Context<BrowserHome>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://example.com/")
                .default_value(value)
        });
        let select = cx.new(|cx| {
            SelectState::new(
                presets
                    .iter()
                    .cloned()
                    .map(|preset| StartupTabOption { preset })
                    .collect::<Vec<_>>(),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        if let Some(preset) = presets.iter().find(|preset| preset.url == value) {
            select.update(cx, |state, cx| state.set_selected_value(preset, window, cx));
        }
        let target = input.clone();
        let select_subscription = cx.subscribe_in(
            &select,
            window,
            move |_, _, event: &SelectEvent<Vec<StartupTabOption>>, window, cx| {
                if let SelectEvent::Confirm(Some(preset)) = event {
                    target.update(cx, |input, cx| {
                        input.set_value(preset.url.clone(), window, cx)
                    });
                }
            },
        );
        let choices = presets.to_vec();
        let target = select.clone();
        let input_subscription = cx.subscribe_in(
            &input,
            window,
            move |_, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value();
                    let matching = choices.iter().find(|preset| preset.url == value.trim());
                    target.update(cx, |state, cx| {
                        if let Some(preset) = matching {
                            state.set_selected_value(preset, window, cx);
                        } else {
                            state.set_selected_index(None, window, cx);
                        }
                    });
                }
            },
        );
        Self {
            input,
            select,
            _subscriptions: vec![select_subscription, input_subscription],
        }
    }
}

impl BrowserHome {
    pub(super) fn render_startup_tab_input(&self, field: &StartupTabField) -> AnyElement {
        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(div().w_full().flex_1().child(Input::new(&field.input)))
            .child(
                div().w_40().flex_shrink_0().child(
                    Select::new(&field.select)
                        .placeholder("常用启动标签页")
                        .accessibility_label("选择常用启动标签页")
                        .search_placeholder("搜索名称或网址")
                        .disabled(self.tab_presets.presets.is_empty())
                        .w_full(),
                ),
            )
            .into_any_element()
    }

    fn reset_tab_preset_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tab_preset_edit = None;
        for input in [&self.tab_preset_name, &self.tab_preset_url] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    fn save_tab_presets(&mut self, settings: BrowserTabSettings, cx: &mut Context<Self>) -> bool {
        match settings.normalized().and_then(|settings| {
            settings.save(self.service.data_dir())?;
            Ok(settings)
        }) {
            Ok(settings) => {
                self.tab_presets = settings;
                self.tab_presets_status = "已保存".into();
                self.tab_presets_error = false;
                cx.notify();
                true
            }
            Err(error) => {
                self.tab_presets_status = error.to_string();
                self.tab_presets_error = true;
                cx.notify();
                false
            }
        }
    }

    fn save_tab_preset_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let preset = BrowserTabPreset {
            name: self.tab_preset_name.read(cx).value().to_string(),
            url: self.tab_preset_url.read(cx).value().to_string(),
        };
        let mut settings = self.tab_presets.clone();
        if let Some(index) = self.tab_preset_edit {
            let Some(existing) = settings.presets.get_mut(index) else {
                return;
            };
            *existing = preset;
        } else {
            settings.presets.push(preset);
        }
        if self.save_tab_presets(settings, cx) {
            self.reset_tab_preset_editor(window, cx);
        }
    }

    pub(super) fn render_browser_tab_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().w_full();
        for (index, preset) in self.tab_presets.presets.iter().enumerate() {
            rows = rows.child(
                h_flex()
                    .w_full()
                    .p_4()
                    .gap_3()
                    .items_center()
                    .when(index > 0, |row| row.border_t_1().border_color(rgb(LINE)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child(preset.name.clone()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child(preset.url.clone()),
                            ),
                    )
                    .child(
                        management_row_button(("edit-tab-preset", index), "编辑").on_click(
                            cx.listener(move |this, _, window, cx| {
                                let Some(preset) = this.tab_presets.presets.get(index).cloned()
                                else {
                                    return;
                                };
                                this.tab_preset_edit = Some(index);
                                this.tab_preset_name.update(cx, |input, cx| {
                                    input.set_value(preset.name, window, cx)
                                });
                                this.tab_preset_url.update(cx, |input, cx| {
                                    input.set_value(preset.url, window, cx)
                                });
                                this.tab_presets_status.clear();
                                this.tab_presets_error = false;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        management_row_button(("remove-tab-preset", index), "删除").on_click(
                            cx.listener(move |this, _, window, cx| {
                                let mut settings = this.tab_presets.clone();
                                if index >= settings.presets.len() {
                                    return;
                                }
                                settings.presets.remove(index);
                                if this.save_tab_presets(settings, cx) {
                                    match this.tab_preset_edit {
                                        Some(editing) if editing == index => {
                                            this.reset_tab_preset_editor(window, cx)
                                        }
                                        Some(editing) if editing > index => {
                                            this.tab_preset_edit = Some(editing - 1)
                                        }
                                        _ => {}
                                    }
                                }
                            }),
                        ),
                    ),
            );
        }
        v_flex()
            .w_full()
            .gap_4()
            .child(div().text_2xl().font_semibold().text_color(rgb(INK)).child("浏览器标签页"))
            .child(div().text_sm().text_color(rgb(MUTED)).child("配置常用启动网址，新建或编辑浏览器时可下拉选择，也可手动输入。修改常用项不会更改已有浏览器的标签页。"))
            .child(
                v_flex().w_full().p_4().gap_3().rounded_md().border_1().border_color(rgb(LINE)).bg(rgb(0xffffff))
                    .child(div().font_semibold().text_color(rgb(INK)).child(if self.tab_preset_edit.is_some() { "编辑常用标签页" } else { "新增常用标签页" }))
                    .child(form_field("名称", &self.tab_preset_name, cx))
                    .child(form_field("网址", &self.tab_preset_url, cx))
                    .child(h_flex().gap_2()
                        .child(Button::new("save-tab-preset").primary().label(if self.tab_preset_edit.is_some() { "保存修改" } else { "添加" })
                            .on_click(cx.listener(|this, _, window, cx| this.save_tab_preset_editor(window, cx))))
                        .when(self.tab_preset_edit.is_some(), |row| row.child(Button::new("cancel-tab-preset").outline().label("取消")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reset_tab_preset_editor(window, cx);
                                this.tab_presets_status.clear();
                                cx.notify();
                            }))))),
            )
            .when(!self.tab_presets_status.is_empty(), |page| page.child(div().text_sm().text_color(rgb(if self.tab_presets_error { RED } else { GREEN })).child(self.tab_presets_status.clone())))
            .child(
                v_flex().w_full().rounded_md().border_1().border_color(rgb(LINE)).bg(rgb(0xffffff))
                    .child(if self.tab_presets.presets.is_empty() {
                        div().p_6().text_sm().text_color(rgb(MUTED)).child("暂无常用标签页，可在上方添加").into_any_element()
                    } else { rows.into_any_element() }),
            )
            .into_any_element()
    }
}
