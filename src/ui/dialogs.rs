use super::*;

impl BrowserHome {
    pub(super) fn confirm_delete_profile(
        &self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let id_for_delete = id.clone();
            let entity = entity.clone();
            alert
                .title("删除浏览器")
                .description(format!("永久删除 {id} 及其浏览器数据？"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("删除")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        if !this.busy {
                            this.delete_profile(id_for_delete.clone(), cx);
                        }
                    });
                    true
                })
        });
    }

    pub(super) fn confirm_close_all(&self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.running.len();
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let entity = entity.clone();
            alert
                .title("关闭全部浏览器")
                .description(format!("关闭当前运行的 {count} 个浏览器？"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("关闭全部")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.close_browsers(this.running.iter().cloned().collect(), cx);
                    });
                    true
                })
        });
    }

    pub(super) fn confirm_remove_managed(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.managed_selected.clone() else {
            return;
        };
        let name = self
            .managed
            .iter()
            .find(|proxy| proxy.id == id)
            .map(|proxy| proxy.name.clone())
            .unwrap_or(id);
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let entity = entity.clone();
            alert
                .title("删除代理")
                .description(format!("删除代理规则“{name}”？"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("删除")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| this.remove_managed(window, cx));
                    true
                })
        });
    }

    pub(super) fn confirm_remove_tag(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tag) = self.selected_tag.clone() else {
            return;
        };
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let entity = entity.clone();
            alert
                .title("删除标签")
                .description(format!("从目录中删除“{tag}”？浏览器已保存的标签会保留。"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("删除")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.remove_tag(cx));
                    true
                })
        });
    }

    pub(super) fn confirm_delete_browser_version(
        &self,
        version: camoufox_pkgman::version::CamoufoxVersion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = version.full_string();
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let version = version.clone();
            let entity = entity.clone();
            alert
                .title("删除浏览器版本")
                .description(format!("删除已安装的历史版本 {label}？"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("删除")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.delete_browser_version(version.clone(), cx);
                    });
                    true
                })
        });
    }

    pub(super) fn render_create_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let smart = self.create_mode == CreateMode::Smart;
        let geo_content: AnyElement = if smart {
            v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(100.))
                                .text_sm()
                                .font_semibold()
                                .text_color(rgb(INK))
                                .child("地理位置"),
                        )
                        .child(
                            Button::new("detect-geo-smart")
                                .outline()
                                .small()
                                .label(if self.geo_loading {
                                    "获取中…"
                                } else {
                                    "自动获取"
                                })
                                .disabled(self.geo_loading)
                                .on_click(cx.listener(|this, _, _, cx| this.detect_geo(cx))),
                        )
                        .child(if self.geo_preview.is_some() {
                            div()
                                .text_sm()
                                .text_color(rgb(GREEN))
                                .child("● 已获取出口位置")
                        } else {
                            div()
                        }),
                )
                .child(if let Some(geo) = &self.geo_preview {
                    v_flex()
                        .gap_2()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(GREEN_PALE))
                        .child(div().font_semibold().text_color(rgb(INK)).child(format!(
                            "{} · {}  /  {}",
                            geo.country,
                            geo.city.as_deref().unwrap_or("-"),
                            geo.ip
                        )))
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .text_color(rgb(GREEN))
                                .child("自动生成的配置"),
                        )
                        .child(geo_summary_line(
                            "国家 / 地区",
                            format!("{} · {}", geo.country_code, geo.country),
                        ))
                        .child(geo_summary_line(
                            "省 / 州",
                            geo.region.clone().unwrap_or_else(|| "-".into()),
                        ))
                        .child(geo_summary_line(
                            "城市",
                            geo.city.clone().unwrap_or_else(|| "-".into()),
                        ))
                        .child(geo_summary_line("时区", geo.timezone.clone()))
                        .child(geo_summary_line("语言", geo.locale.clone()))
                        .child(geo_summary_line(
                            "经纬度",
                            format!("{:.4} / {:.4}", geo.latitude, geo.longitude),
                        ))
                        .into_any_element()
                } else {
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(ROW_ALT))
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child(if self.geo_loading {
                            "正在通过代理获取出口位置…"
                        } else {
                            "自动获取后，这里会显示地理位置配置。"
                        })
                        .into_any_element()
                })
                .into_any_element()
        } else {
            v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .items_center()
                        .child(
                            div()
                                .text_sm()
                                .font_semibold()
                                .text_color(rgb(INK))
                                .child("地理位置"),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("detect-geo-custom")
                                .outline()
                                .small()
                                .label(if self.geo_loading {
                                    "获取中…"
                                } else {
                                    "自动获取"
                                })
                                .disabled(self.geo_loading)
                                .on_click(cx.listener(|this, _, _, cx| this.detect_geo(cx))),
                        ),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .child(geo_field("国家代码", &self.form_country_code))
                        .child(geo_field("国家 / 地区", &self.form_country)),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .child(geo_field("省 / 州", &self.form_region))
                        .child(geo_field("城市", &self.form_city)),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .child(geo_field("时区", &self.form_timezone))
                        .child(geo_field("语言", &self.form_locale)),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .child(geo_field("纬度", &self.form_latitude))
                        .child(geo_field("经度", &self.form_longitude)),
                )
                .into_any_element()
        };
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .p_1()
                    .rounded_md()
                    .bg(rgb(ROW_ALT))
                    .child(
                        Button::new("create-mode-smart")
                            .flex_1()
                            .when(smart, |button| button.primary())
                            .when(!smart, |button| button.ghost())
                            .label("智能模式")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create_mode = CreateMode::Smart;
                                if let Some(geo) = this.geo_preview.clone() {
                                    this.fill_geo_fields(&geo, window, cx);
                                } else if !this.geo_loading {
                                    this.detect_geo(cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("create-mode-custom")
                            .flex_1()
                            .when(!smart, |button| button.primary())
                            .when(smart, |button| button.ghost())
                            .label("自定义模式")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.create_mode = CreateMode::Custom;
                                cx.notify();
                            })),
                    ),
            )
            .child(form_field("名称", &self.form_name, cx))
            .child(self.render_tabs_field(cx))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(100.))
                            .text_sm()
                            .text_color(rgb(INK))
                            .child("网络代理"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Select::new(&self.form_proxy_select).w_full()),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(100.))
                            .text_sm()
                            .text_color(rgb(INK))
                            .child("操作系统"),
                    )
                    .child(
                        h_flex().gap_2().children(
                            [
                                (ProfileOs::Windows, "Windows", "create-os-windows"),
                                (ProfileOs::Macos, "macOS", "create-os-macos"),
                                (ProfileOs::Linux, "Linux", "create-os-linux"),
                            ]
                            .into_iter()
                            .map(|(os, label, id)| {
                                Button::new(id)
                                    .w(px(92.))
                                    .small()
                                    .when(
                                        std::mem::discriminant(&self.form_os)
                                            == std::mem::discriminant(&os),
                                        |button| button.primary(),
                                    )
                                    .when(
                                        std::mem::discriminant(&self.form_os)
                                            != std::mem::discriminant(&os),
                                        |button| button.outline(),
                                    )
                                    .label(label)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.form_os = os;
                                        cx.notify();
                                    }))
                            }),
                        ),
                    ),
            )
            .child(div().h(px(1.)).w_full().bg(rgb(LINE)))
            .child(geo_content)
            .child(if self.geo_error.is_empty() {
                div().into_any_element()
            } else {
                div()
                    .text_sm()
                    .text_color(rgb(RED))
                    .child(self.geo_error.clone())
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_tabs_field(&self, cx: &mut Context<Self>) -> AnyElement {
        let extra_rows = self
            .form_extra_tabs
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, input)| {
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(100.)))
                    .child(div().flex_1().child(Input::new(&input)))
                    .child(
                        Button::new(("remove-startup-tab", index))
                            .outline()
                            .small()
                            .w(px(72.))
                            .label("移除")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.form_extra_tabs.remove(index);
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(100.))
                            .text_sm()
                            .text_color(rgb(INK))
                            .child("标签页"),
                    )
                    .child(div().flex_1().child(Input::new(&self.form_url)))
                    .child(
                        Button::new("add-startup-tab")
                            .outline()
                            .small()
                            .w(px(72.))
                            .label("添加")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.form_extra_tabs.push(cx.new(|cx| {
                                    InputState::new(window, cx).placeholder("https://example.com/")
                                }));
                                cx.notify();
                            })),
                    ),
            )
            .children(extra_rows)
            .into_any_element()
    }

    pub(super) fn render_dialog(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let dialog = self.dialog.clone();
        if matches!(dialog, Dialog::None) {
            return div().into_any_element();
        }
        let title = match &dialog {
            Dialog::Create => "新建浏览器",
            Dialog::Edit(_) => "编辑浏览器",
            Dialog::None => unreachable!(),
        };
        let body: AnyElement = match dialog.clone() {
            Dialog::Create => self.render_create_body(cx),
            Dialog::Edit(id) => v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(100.)).text_sm().text_color(rgb(INK)).child("ID"))
                        .child(div().text_sm().text_color(rgb(MUTED)).child(id)),
                )
                .child(form_field("名称", &self.form_name, cx))
                .child(self.render_tabs_field(cx))
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(100.)).text_sm().child("网络代理"))
                        .child(
                            div()
                                .flex_1()
                                .child(Select::new(&self.form_proxy_select).w_full()),
                        ),
                )
                .child(div().text_xs().text_color(rgb(MUTED)).child(
                    "修改代理后，启动时仍会校验出口国家和时区；已固定的浏览器指纹保持不变。",
                ))
                .into_any_element(),
            Dialog::None => unreachable!(),
        };
        let confirm = match dialog {
            Dialog::Create | Dialog::Edit(_) => "保存",
            Dialog::None => unreachable!(),
        };
        let confirm_dialog = dialog.clone();
        let confirm_disabled = self.busy
            || (matches!(dialog, Dialog::Create)
                && self.create_mode == CreateMode::Smart
                && (self.geo_loading || self.geo_preview.is_none()));
        let card =
            v_flex()
                .w(px(if matches!(dialog, Dialog::Create) {
                    640.
                } else {
                    520.
                }))
                .max_h(window.bounds().size.height - px(48.))
                .p_5()
                .gap_4()
                .rounded_lg()
                .bg(rgb(0xffffff))
                .shadow_lg()
                .child(
                    h_flex()
                        .items_center()
                        .child(
                            div()
                                .text_lg()
                                .font_semibold()
                                .text_color(rgb(INK))
                                .child(title),
                        )
                        .child(div().flex_1())
                        .child(Button::new("dialog-close").ghost().label("×").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.dialog = Dialog::None;
                                cx.notify();
                            }),
                        )),
                )
                .child(
                    div()
                        .max_h(window.bounds().size.height - px(190.))
                        .overflow_y_scrollbar()
                        .child(body),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("dialog-cancel")
                                .outline()
                                .w(px(88.))
                                .label("取消")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.dialog = Dialog::None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("dialog-confirm")
                                .primary()
                                .w(px(88.))
                                .label(confirm)
                                .disabled(confirm_disabled)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    match &confirm_dialog {
                                        Dialog::Create | Dialog::Edit(_) => {
                                            this.submit_form(window, cx)
                                        }
                                        Dialog::None => {}
                                    }
                                })),
                        ),
                );
        div()
            .absolute()
            .inset_0()
            .bg(hsla(0., 0., 0., 0.38))
            .flex()
            .items_center()
            .justify_center()
            .child(card)
            .into_any_element()
    }

    pub(super) fn render_launch_dialog(
        &self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(launch) = self.launch.as_ref().filter(|launch| launch.visible) else {
            return div().into_any_element();
        };
        let steps = launch.steps();
        let active = launch
            .stage
            .and_then(|stage| steps.iter().position(|item| *item == stage))
            .unwrap_or(0);
        let failed = launch.error.is_some();
        let fill = ((active as f32 + 0.5) / steps.len() as f32).clamp(0.08, 0.94);
        let id = launch.id.clone();
        let items = steps
            .iter()
            .enumerate()
            .map(|(index, stage)| {
                let completed = index < active;
                let current = index == active;
                let color = if failed && current {
                    RED
                } else if completed || current {
                    GREEN
                } else {
                    MUTED
                };
                let marker = div()
                    .size(px(25.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_2()
                    .border_color(rgb(color))
                    .bg(rgb(if completed { GREEN } else { 0xffffff }))
                    .text_xs()
                    .font_semibold()
                    .text_color(rgb(0xffffff))
                    .child(if completed {
                        "✓"
                    } else if failed && current {
                        "×"
                    } else {
                        ""
                    });
                h_flex()
                    .min_h(px(32.))
                    .items_center()
                    .gap_3()
                    .child(marker)
                    .child(
                        v_flex().child(
                            div()
                                .text_sm()
                                .font_semibold()
                                .text_color(rgb(if current || completed { INK } else { MUTED }))
                                .child(launch_stage_label(*stage)),
                        ),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let card = v_flex()
            .w(px(430.))
            .rounded_lg()
            .border_1()
            .border_color(rgb(LINE))
            .bg(rgb(0xffffff))
            .shadow_lg()
            .child(
                v_flex()
                    .p_4()
                    .gap_3()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(img(app_logo()).size(px(38.)).object_fit(ObjectFit::Contain))
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div().text_lg().font_semibold().text_color(rgb(INK)).child(
                                            if failed {
                                                "打开浏览器失败"
                                            } else {
                                                "正在打开浏览器"
                                            },
                                        ),
                                    )
                                    .child(
                                        div().text_sm().text_color(rgb(MUTED)).child(format!(
                                            "{} · {}",
                                            launch.name, launch.location
                                        )),
                                    ),
                            ),
                    )
                    .child(v_flex().gap_2().children(items))
                    .child(
                        div().w_full().h(px(7.)).rounded_full().bg(rgb(LINE)).child(
                            div()
                                .w(relative(fill))
                                .h_full()
                                .rounded_full()
                                .bg(rgb(if failed { RED } else { GREEN })),
                        ),
                    )
                    .child(if let Some(error) = &launch.error {
                        div()
                            .p_3()
                            .rounded_md()
                            .bg(rgb(0xfff1f0))
                            .text_sm()
                            .text_color(rgb(RED))
                            .child(launch_error_message(error))
                            .into_any_element()
                    } else {
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("请稍候，首次启动可能需要更长时间")
                            .into_any_element()
                    }),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_3()
                    .p_4()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .child(
                        Button::new("launch-cancel")
                            .outline()
                            .label(if failed { "关闭" } else { "取消启动" })
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_launch(cx))),
                    )
                    .child(if failed {
                        Button::new("launch-retry")
                            .primary()
                            .label("重试")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.cancel_launch(cx);
                                this.open_browser(id.clone(), window, cx);
                            }))
                            .into_any_element()
                    } else {
                        Button::new("launch-background")
                            .ghost()
                            .label("在后台继续")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(launch) = this.launch.as_mut() {
                                    launch.visible = false;
                                    cx.notify();
                                }
                            }))
                            .into_any_element()
                    }),
            );
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(card)
            .into_any_element()
    }
}
