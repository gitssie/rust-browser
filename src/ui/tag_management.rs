use super::*;

impl BrowserHome {
    pub(super) fn render_tag_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut tags = v_flex();
        for (index, tag) in self.all_tags.iter().enumerate() {
            let name = tag.clone();
            let edit_name = name.clone();
            let delete_name = name.clone();
            let color = self
                .tag_custom_colors
                .get(&tag.to_lowercase())
                .and_then(|value| try_parse_color(value).ok())
                .unwrap_or_else(|| rgb(tag_colors(tag).1).into());
            tags = tags.child(
                h_flex()
                    .h(px(60.))
                    .px_4()
                    .gap_3()
                    .items_center()
                    .when(index > 0, |row| row.border_t_1().border_color(rgb(LINE)))
                    .child(div().size(px(18.)).rounded_full().bg(color))
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child(name),
                    )
                    .child(
                        management_row_button(format!("edit-catalog-tag-{edit_name}"), "编辑")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_tag_editor(Some(edit_name.clone()), window, cx)
                            })),
                    )
                    .child(
                        management_row_button(format!("delete-catalog-tag-{delete_name}"), "删除")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.selected_tag = Some(delete_name.clone());
                                this.confirm_remove_tag(window, cx);
                            })),
                    ),
            );
        }
        v_flex()
            .w_full()
            .gap_4()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_2xl()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("标签管理"),
                    )
                    .child(
                        management_add_button("new-catalog-tag", "新增标签").on_click(cx.listener(
                            |this, _, window, cx| this.open_tag_editor(None, window, cx),
                        )),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(LINE))
                    .overflow_hidden()
                    .bg(rgb(0xffffff))
                    .child(if self.all_tags.is_empty() {
                        div()
                            .h(px(120.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child("还没有标签，点击右上角新增")
                            .into_any_element()
                    } else {
                        tags.into_any_element()
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_tag_editor(&self) -> AnyElement {
        v_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("名称"),
                    )
                    .child(Input::new(&self.tag_input)),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("颜色"),
                    )
                    .child(
                        ColorPicker::new(&self.tag_color_picker)
                            .featured_colors(vec![rgb(GREEN).into()])
                            .accessibility_label("标签颜色"),
                    ),
            )
            .when(!self.tag_form_error.is_empty(), |form| {
                form.child(
                    div()
                        .text_sm()
                        .text_color(rgb(RED))
                        .child(self.tag_form_error.clone()),
                )
            })
            .into_any_element()
    }

    fn open_tag_editor(
        &mut self,
        tag: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_tag = tag.clone();
        let name = tag.clone().unwrap_or_default();
        self.tag_input
            .update(cx, |input, cx| input.set_value(&name, window, cx));
        let color = tag
            .as_ref()
            .and_then(|name| self.tag_custom_colors.get(&name.to_lowercase()))
            .and_then(|value| try_parse_color(value).ok())
            .or_else(|| tag.as_ref().map(|name| rgb(tag_colors(name).1).into()))
            .unwrap_or_else(|| random_tag_color(&self.tag_custom_colors));
        self.tag_color_picker
            .update(cx, |picker, cx| picker.set_value(color, window, cx));
        self.tag_form_error.clear();
        let home = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let content_home = home.clone();
            let save_home = home.clone();
            dialog
                .title(if tag.is_some() {
                    "编辑标签"
                } else {
                    "新增标签"
                })
                .w(px(440.))
                .content(move |content, _, cx| {
                    content.child(content_home.read(cx).render_tag_editor())
                })
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("cancel-tag-editor")
                                .outline()
                                .label("取消")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("save-tag-editor")
                                .primary()
                                .label("保存标签")
                                .on_click(move |_, window, cx| {
                                    if save_home.update(cx, |this, cx| this.save_tag(cx)) {
                                        window.close_dialog(cx);
                                    }
                                }),
                        ),
                )
        });
    }

    fn save_tag(&mut self, cx: &mut Context<Self>) -> bool {
        let name = self.tag_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.tag_form_error = "请输入标签名称".into();
            cx.notify();
            return false;
        }
        if self.all_tags.iter().any(|existing| {
            existing.eq_ignore_ascii_case(&name)
                && self
                    .selected_tag
                    .as_deref()
                    .is_none_or(|old| !existing.eq_ignore_ascii_case(old))
        }) {
            self.tag_form_error = "标签名称已存在，请更换名称".into();
            cx.notify();
            return false;
        }
        let color = self
            .tag_color_picker
            .read(cx)
            .value()
            .unwrap_or_else(|| rgb(GREEN).into())
            .to_hex();
        let catalog = TagCatalog::new(self.service.data_dir());
        let result = if let Some(old) = &self.selected_tag {
            catalog.update(old, &name, &color)
        } else {
            catalog.add_with_color(&name, &color)
        };
        match result {
            Ok(()) => {
                self.selected_tag = None;
                self.tag_form_error.clear();
                self.pending_notice = Some(Notification::info("标签已保存"));
                self.load_tags(cx);
                cx.notify();
                true
            }
            Err(error) => {
                self.tag_form_error = format!("保存失败：{error}");
                cx.notify();
                false
            }
        }
    }

    pub(super) fn remove_tag(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.selected_tag.clone() else {
            return;
        };
        match TagCatalog::new(self.service.data_dir()).remove(&name) {
            Ok(()) => {
                self.selected_tag = None;
                self.pending_notice = Some(Notification::info("标签已删除"));
                self.load_tags(cx);
            }
            Err(error) => {
                self.pending_notice = Some(Notification::error(format!("删除失败：{error}")))
            }
        }
        cx.notify();
    }
    pub(super) fn load_tags(&mut self, cx: &mut Context<Self>) {
        match TagCatalog::new(self.service.data_dir()).list_entries() {
            Ok(tags) => {
                self.tag_custom_colors = tags
                    .iter()
                    .filter_map(|tag| {
                        tag.color
                            .as_ref()
                            .map(|color| (tag.name.to_lowercase(), color.clone()))
                    })
                    .collect();
                self.all_tags = tags.into_iter().map(|tag| tag.name).collect();
            }
            Err(error) => {
                self.pending_notice = Some(Notification::error(format!("读取标签失败：{error}")))
            }
        }
        self.filter_tags_generation += 1;
        let generation = self.filter_tags_generation;
        let service = self.service.clone();
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { tokio.block_on(service.list_tags()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.filter_tags_generation != generation {
                    return;
                }
                match result {
                    Ok(profile_tags) => {
                        let mut options = this.all_tags.clone();
                        for tag in profile_tags {
                            if !options
                                .iter()
                                .any(|option| option.eq_ignore_ascii_case(&tag))
                            {
                                options.push(tag);
                            }
                        }
                        options.sort_by_key(|tag| tag.to_lowercase());
                        this.filter_tag_options = options;
                    }
                    Err(error) => {
                        this.pending_notice =
                            Some(Notification::error(format!("读取筛选标签失败：{error}")))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn add_tag(&mut self, profile_id: String, cx: &mut Context<Self>) {
        let tag = self.tag_input.read(cx).value().trim().to_string();
        if tag.is_empty() {
            return;
        }
        if let Err(error) = TagCatalog::new(self.service.data_dir()).add(&tag) {
            self.pending_notice = Some(Notification::error(format!("添加标签失败：{error}")));
            cx.notify();
            return;
        }
        self.load_tags(cx);
        self.toggle_tag(profile_id, tag, cx);
    }
}
