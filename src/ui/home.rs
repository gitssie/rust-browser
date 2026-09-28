use super::*;

impl BrowserHome {
    pub(super) fn visible_row_indices(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, profile)| {
                self.status_filter
                    .is_none_or(|running| self.running.contains(&profile.id) == running)
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub(super) fn render_search_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let can_close_all = !self.running.is_empty();
        let status_label = match self.status_filter {
            None => "全部",
            Some(true) => "运行中",
            Some(false) => "未运行",
        };
        let tag_label = if self.filter_tags.is_empty() {
            "全部".to_string()
        } else {
            format!("已选 {}", self.filter_tags.len())
        };
        let status_entity = cx.entity();
        let proxy_entity = cx.entity();
        let tag_entity = cx.entity();
        let status_filter = self.status_filter;
        let proxy_filter = self.proxy_filter.clone();
        let proxy_label = proxy_filter
            .as_ref()
            .map(|choice| proxy_display_name(choice, &self.managed))
            .unwrap_or_else(|| "全部".into());
        let managed_proxies = self.managed.clone();
        let all_tags = self.filter_tag_options.clone();
        let selected_tags = self.filter_tags.clone();
        h_flex()
            .h(px(40.))
            .items_center()
            .gap_2()
            .child(div().w(px(235.)).mr_1().child(
                Input::new(&self.search).prefix(Icon::new(IconName::Search).text_color(rgb(MUTED))),
            ))
            .child(
                Popover::new("status-filter-popover")
                    .trigger(
                        Button::new("status-filter")
                            .outline()
                            .small()
                            .h(px(34.))
                            .label(format!("状态   {status_label}"))
                            .dropdown_caret(true),
                    )
                    .content(move |_, _, cx| {
                        let popover = cx.entity();
                        v_flex().w(px(150.)).gap_1().children(
                            [
                                (None, "全部"),
                                (Some(true), "运行中"),
                                (Some(false), "未运行"),
                            ]
                            .into_iter()
                            .map(|(value, label)| {
                                let entity = status_entity.clone();
                                let popover = popover.clone();
                                h_flex()
                                    .id(SharedString::from(format!("status-choice-{label}")))
                                    .role(Role::Button)
                                    .aria_label(format!("状态：{label}"))
                                    .h(px(32.))
                                    .px_2()
                                    .items_center()
                                    .justify_between()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .when(status_filter == value, |row| row.bg(rgb(SELECTED)))
                                    .child(label)
                                    .child(if status_filter == value { "✓" } else { "" })
                                    .on_click(move |_, window, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.status_filter = value;
                                            cx.notify();
                                        });
                                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                                    })
                            }),
                        )
                    }),
            )
            .child(
                Popover::new("proxy-filter-popover")
                    .trigger(
                        Button::new("proxy-filter")
                            .outline()
                            .small()
                            .h(px(34.))
                            .label(format!("代理   {proxy_label}"))
                            .dropdown_caret(true),
                    )
                    .content(move |_, _, cx| {
                        let popover = cx.entity();
                        let options = std::iter::once((None, "全部".to_string()))
                            .chain(std::iter::once((
                                Some(ProxyChoice::Global),
                                "全局".to_string(),
                            )))
                            .chain(std::iter::once((
                                Some(ProxyChoice::Direct),
                                "直连".to_string(),
                            )))
                            .chain(managed_proxies.iter().map(|proxy| {
                                (
                                    Some(ProxyChoice::Custom(proxy.url.clone())),
                                    proxy.name.clone(),
                                )
                            }))
                            .collect::<Vec<_>>();
                        v_flex()
                            .w(px(190.))
                            .max_h(px(320.))
                            .overflow_y_scrollbar()
                            .gap_1()
                            .children(options.into_iter().enumerate().map(
                                |(index, (choice, label))| {
                                    let entity = proxy_entity.clone();
                                    let popover = popover.clone();
                                    let selected = proxy_filter == choice;
                                    h_flex()
                                        .id(SharedString::from(format!("proxy-choice-{index}")))
                                        .role(Role::Button)
                                        .aria_label(format!("代理：{label}"))
                                        .h(px(32.))
                                        .px_2()
                                        .items_center()
                                        .justify_between()
                                        .rounded_sm()
                                        .cursor_pointer()
                                        .when(selected, |row| row.bg(rgb(SELECTED)))
                                        .child(label)
                                        .child(if selected { "✓" } else { "" })
                                        .on_click(move |_, window, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.proxy_filter = choice.clone();
                                                this.reload(cx);
                                            });
                                            popover
                                                .update(cx, |state, cx| state.dismiss(window, cx));
                                        })
                                },
                            ))
                    }),
            )
            .child(
                Popover::new("tag-filter-popover")
                    .trigger(
                        Button::new("tag-filter")
                            .outline()
                            .small()
                            .h(px(34.))
                            .label(format!("标签   {tag_label}"))
                            .dropdown_caret(true),
                    )
                    .content(move |_, _, _| {
                        let mut menu = v_flex()
                            .w(px(210.))
                            .max_h(px(320.))
                            .overflow_y_scrollbar()
                            .gap_2();
                        let clear_entity = tag_entity.clone();
                        menu = menu.child(
                            Button::new("clear-tag-filter")
                                .ghost()
                                .small()
                                .label("全部标签")
                                .on_click(move |_, _, cx| {
                                    clear_entity.update(cx, |this, cx| {
                                        this.filter_tags.clear();
                                        this.reload(cx);
                                    });
                                }),
                        );
                        if all_tags.is_empty() {
                            menu = menu
                                .child(div().text_sm().text_color(rgb(MUTED)).child("暂无标签"));
                        }
                        for tag in &all_tags {
                            let entity = tag_entity.clone();
                            let tag_for_click = tag.clone();
                            menu =
                                menu.child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            Checkbox::new(SharedString::from(format!(
                                                "filter-tag-{tag}"
                                            )))
                                            .checked(selected_tags.contains(tag))
                                            .accessibility_label(format!("筛选标签 {tag}"))
                                            .on_click(move |checked, _, cx| {
                                                entity.update(cx, |this, cx| {
                                                    if *checked {
                                                        if !this
                                                            .filter_tags
                                                            .contains(&tag_for_click)
                                                        {
                                                            this.filter_tags
                                                                .push(tag_for_click.clone());
                                                        }
                                                    } else {
                                                        this.filter_tags.retain(|selected| {
                                                            selected != &tag_for_click
                                                        });
                                                    }
                                                    this.reload(cx);
                                                });
                                            }),
                                        )
                                        .child(div().text_sm().child(tag.clone())),
                                );
                        }
                        menu
                    }),
            )
            .child(div().flex_1())
            .child(div().size(px(7.)).rounded_full().bg(rgb(GREEN)))
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(rgb(INK))
                    .child(format!("运行中 {}", self.running.len())),
            )
            .when(can_close_all, |bar| {
                bar.child(
                    Button::new("close-all")
                        .outline()
                        .small()
                        .border_color(rgb(0xf2b8b5))
                        .text_color(rgb(RED))
                        .label("关闭全部")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.confirm_close_all(window, cx);
                        })),
                )
            })
            .when(
                self.launch.as_ref().is_some_and(|launch| !launch.visible),
                |bar| {
                    bar.child(
                        Button::new("show-launch-progress")
                            .ghost()
                            .small()
                            .label("启动进度")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(launch) = this.launch.as_mut() {
                                    launch.visible = true;
                                    cx.notify();
                                }
                            })),
                    )
                },
            )
            .child(
                h_flex()
                    .id("new-profile")
                    .role(Role::Button)
                    .aria_label("新建浏览器")
                    .h(px(34.))
                    .px_3()
                    .gap_1()
                    .items_center()
                    .rounded_md()
                    .bg(rgb(GREEN))
                    .text_sm()
                    .text_color(rgb(0xffffff))
                    .font_semibold()
                    .cursor_pointer()
                    .child(Icon::new(IconName::Plus))
                    .child("新建浏览器")
                    .on_click(cx.listener(|this, _, window, cx| this.show_create(window, cx))),
            )
            .into_any_element()
    }
}
