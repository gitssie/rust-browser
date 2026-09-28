use super::*;

impl BrowserHome {
    pub(super) fn render_profile_card(
        &self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let profile = self.rows[index].clone();
        let id = profile.id.clone();
        let card_focus = window
            .use_keyed_state(
                SharedString::from(format!("profile-card-focus-{id}")),
                cx,
                |_, cx| cx.focus_handle(),
            )
            .read(cx)
            .clone();
        let focus_inside = card_focus.contains_focused(window, cx);
        let hover_group = SharedString::from(format!("profile-card-hover-{id}"));
        let running = self.running.contains(&id);
        let launching = self
            .launch
            .as_ref()
            .is_some_and(|launch| launch.id == id && launch.error.is_none());
        let run_id = id.clone();
        let edit_id = id.clone();
        let delete_id = id.clone();
        let os = match profile.os.as_str() {
            "macos" => "macOS",
            "windows" => "Windows",
            "linux" => "Linux",
            _ => "未知系统",
        };
        let location = format!(
            "{} · {}",
            profile.saved_geo.country,
            profile.saved_geo.city.as_deref().unwrap_or("-"),
        );
        let flag = country_flag(&profile.saved_geo.country_code);
        let proxy = proxy_display_name(&profile.proxy, &self.managed);
        let tag_count = profile.tags.len();
        let selected_tags = profile.tags.clone();
        let mut available_tags = self.all_tags.clone();
        for tag in &selected_tags {
            if !available_tags
                .iter()
                .any(|known| known.eq_ignore_ascii_case(tag))
            {
                available_tags.push(tag.clone());
            }
        }
        let tag_input = self.tag_input.clone();
        let tag_home = cx.entity();
        let tag_profile_id = id.clone();
        let tag_trigger_id = id.clone();
        let tags = profile
            .tags
            .iter()
            .take(2)
            .cloned()
            .map(|tag| {
                let (background, foreground) = tag_colors(&tag);
                let custom = self
                    .tag_custom_colors
                    .get(&tag.to_lowercase())
                    .and_then(|color| try_parse_color(color).ok());
                div()
                    .h(px(20.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded_sm()
                    .bg(custom
                        .map(|color| color.opacity(0.13))
                        .unwrap_or_else(|| rgb(background).into()))
                    .text_xs()
                    .text_color(custom.unwrap_or_else(|| rgb(foreground).into()))
                    .child(tag)
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        v_flex()
            .id(SharedString::from(format!("profile-card-{id}")))
            .flex_1()
            .min_w_0()
            .h(px(150.))
            .p_3()
            .gap_1()
            .rounded_md()
            .border_1()
            .border_color(rgb(LINE))
            .bg(rgb(0xffffff))
            .group(hover_group.clone())
            .track_focus(&card_focus)
            .tab_group()
            .tab_stop(true)
            .child(
                h_flex()
                    .h(px(40.))
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .relative()
                            .size(px(36.))
                            .flex_shrink_0()
                            .child(
                                div()
                                    .size_full()
                                    .rounded_md()
                                    .bg(rgb(GREEN))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .font_semibold()
                                    .text_color(rgb(0xffffff))
                                    .child(
                                        profile
                                            .name
                                            .as_deref()
                                            .unwrap_or(&profile.id)
                                            .chars()
                                            .next()
                                            .unwrap_or('B')
                                            .to_uppercase()
                                            .to_string(),
                                    ),
                            )
                            .when(running, |avatar| {
                                avatar.child(
                                    div()
                                        .absolute()
                                        .top(px(-4.))
                                        .left(px(-4.))
                                        .size(px(13.))
                                        .rounded_full()
                                        .border_2()
                                        .border_color(rgb(0xffffff))
                                        .bg(rgb(GREEN)),
                                )
                            }),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .text_ellipsis()
                                    .child(
                                        profile.name.clone().unwrap_or_else(|| profile.id.clone()),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(
                                        div()
                                            .max_w(px(95.))
                                            .text_ellipsis()
                                            .child(profile.id.clone()),
                                    )
                                    .child(if os == "Windows" {
                                        svg()
                                            .data(WINDOWS_ICON)
                                            .size(px(15.))
                                            .text_color(rgb(0x087cf2))
                                            .into_any_element()
                                    } else if os == "macOS" {
                                        div().text_size(px(16.)).child("").into_any_element()
                                    } else {
                                        div().text_size(px(13.)).child("◆").into_any_element()
                                    })
                                    .child(div().h(px(13.)).w(px(1.)).bg(rgb(LINE)))
                                    .child(div().max_w(px(85.)).text_ellipsis().child(proxy)),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("run-{id}")))
                            .outline()
                            .xsmall()
                            .h(px(30.))
                            .w(px(30.))
                            .loading(launching)
                            .accessibility_label(if launching {
                                "正在启动浏览器"
                            } else if running {
                                "关闭浏览器"
                            } else {
                                "启动浏览器"
                            })
                            .tooltip(if launching {
                                "正在启动浏览器"
                            } else if running {
                                "关闭浏览器"
                            } else {
                                "启动浏览器"
                            })
                            .child(if launching {
                                Spinner::new()
                                    .with_size(gpui_component::Size::Small)
                                    .color(rgb(GREEN).into())
                                    .into_any_element()
                            } else {
                                svg()
                                    .data(if running { POWER_ICON } else { PLAY_ICON })
                                    .size(px(17.))
                                    .text_color(rgb(if running { RED } else { GREEN }))
                                    .into_any_element()
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if running {
                                    this.close_browsers(vec![run_id.clone()], cx);
                                } else {
                                    this.open_browser(run_id.clone(), window, cx);
                                }
                            })),
                    ),
            )
            .child(
                h_flex()
                    .h(px(22.))
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .text_color(rgb(0x4c618b))
                    .child(div().child(flag))
                    .child(div().flex_1().min_w_0().text_ellipsis().child(location)),
            )
            .child(
                gpui_base::Popover::new(SharedString::from(format!("profile-tags-{id}")))
                    .trigger_with(move |_, _, _| {
                        h_flex()
                            .id(SharedString::from(format!("tag-cell-{tag_trigger_id}")))
                            .role(Role::Button)
                            .aria_label(format!("编辑 {} 的标签", tag_trigger_id))
                            .h(px(24.))
                            .gap_1()
                            .items_center()
                            .cursor_pointer()
                            .text_xs()
                            .child(svg().data(NAV_TAG_ICON).size(px(14.)).text_color(rgb(INK)))
                            .children(tags)
                            .when(tag_count > 2, |row| {
                                row.child(
                                    div()
                                        .text_color(rgb(MUTED))
                                        .child(format!("+{}", tag_count - 2)),
                                )
                            })
                            .child(div().text_color(rgb(0x087cf2)).child("+"))
                            .into_any_element()
                    })
                    .content(move |_, _, _| {
                        div()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(LINE))
                            .bg(rgb(0xffffff))
                            .shadow_lg()
                            .child(render_card_tag_menu(
                                tag_profile_id.clone(),
                                selected_tags.clone(),
                                available_tags.clone(),
                                tag_input.clone(),
                                tag_home.clone(),
                            ))
                    }),
            )
            .child(
                h_flex()
                    .h(px(28.))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(profile.last_opened_at.map(format_date).unwrap_or_default()),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .opacity(if focus_inside { 1. } else { 0. })
                            .group_hover(hover_group, |style| style.opacity(1.))
                            .child(
                                Button::new(SharedString::from(format!("edit-{id}")))
                                    .outline()
                                    .xsmall()
                                    .h(px(24.))
                                    .w(px(24.))
                                    .accessibility_label("编辑配置")
                                    .tooltip("编辑配置")
                                    .child(
                                        svg().data(PENCIL_ICON).size(px(14.)).text_color(rgb(INK)),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.show_edit(edit_id.clone(), window, cx)
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("delete-{id}")))
                                    .outline()
                                    .xsmall()
                                    .h(px(24.))
                                    .w(px(24.))
                                    .accessibility_label("删除配置")
                                    .tooltip("删除配置")
                                    .child(
                                        svg().data(TRASH_ICON).size(px(14.)).text_color(rgb(RED)),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.confirm_delete_profile(delete_id.clone(), window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn render_card_tag_menu(
    profile_id: String,
    selected_tags: Vec<String>,
    available_tags: Vec<String>,
    tag_input: Entity<InputState>,
    home: Entity<BrowserHome>,
) -> AnyElement {
    let mut menu = v_flex()
        .w(px(250.))
        .max_h(px(360.))
        .overflow_y_scrollbar()
        .p_3()
        .gap_2()
        .child(div().font_semibold().text_color(rgb(INK)).child("选择标签"));
    if available_tags.is_empty() {
        menu = menu.child(div().text_xs().text_color(rgb(MUTED)).child("暂无标签"));
    }
    for tag in available_tags {
        let tag_for_click = tag.clone();
        let id_for_click = profile_id.clone();
        let home_for_click = home.clone();
        menu = menu.child(
            h_flex()
                .h(px(28.))
                .gap_2()
                .items_center()
                .child(
                    Checkbox::new(SharedString::from(format!("card-tag-{profile_id}-{tag}")))
                        .checked(selected_tags.contains(&tag))
                        .accessibility_label(format!("标签 {tag}"))
                        .on_click(move |_, _, cx| {
                            home_for_click.update(cx, |this, cx| {
                                this.toggle_tag(id_for_click.clone(), tag_for_click.clone(), cx)
                            });
                        }),
                )
                .child(div().text_sm().text_color(rgb(INK)).child(tag)),
        );
    }
    let add_id = profile_id.clone();
    menu.child(
        h_flex()
            .pt_2()
            .gap_1()
            .border_t_1()
            .border_color(rgb(LINE))
            .child(div().flex_1().child(Input::new(&tag_input).small()))
            .child(
                Button::new(SharedString::from(format!("card-add-tag-{profile_id}")))
                    .outline()
                    .small()
                    .label("添加")
                    .on_click(move |_, _, cx| {
                        home.update(cx, |this, cx| this.add_tag(add_id.clone(), cx));
                    }),
            ),
    )
    .into_any_element()
}
