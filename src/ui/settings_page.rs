use super::*;

impl BrowserHome {
    pub(super) fn render_navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.settings_tab;
        v_flex()
            .w(px(204.))
            .h_full()
            .flex_shrink_0()
            .p_3()
            .gap_1()
            .border_r_1()
            .border_color(rgb(LINE))
            .bg(rgb(0xffffff))
            .child(
                h_flex()
                    .h(px(56.))
                    .gap_2()
                    .items_center()
                    .pl_2()
                    .mb_3()
                    .child(img(app_logo()).size(px(36.)).object_fit(ObjectFit::Contain))
                    .child(
                        div()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("Cazer Browser"),
                    )
                    .child(div().flex_1()),
            )
            .child(
                h_flex()
                    .id("nav-browsers")
                    .role(Role::Button)
                    .aria_label("浏览器")
                    .h(px(42.))
                    .px_3()
                    .gap_3()
                    .items_center()
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(rgb(if active.is_none() { GREEN } else { INK }))
                    .when(active.is_none(), |item| item.bg(rgb(0xe6f7f1)))
                    .child(
                        svg()
                            .data(NAV_BROWSER_ICON)
                            .size(px(18.))
                            .text_color(rgb(if active.is_none() { GREEN } else { INK })),
                    )
                    .child("浏览器")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_tab = None;
                        this.reload(cx);
                    })),
            )
            .child(
                h_flex()
                    .id("nav-proxies")
                    .role(Role::Button)
                    .aria_label("网络代理")
                    .h(px(42.))
                    .px_3()
                    .gap_3()
                    .items_center()
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(rgb(if active == Some(SettingsTab::Proxies) {
                        GREEN
                    } else {
                        INK
                    }))
                    .when(active == Some(SettingsTab::Proxies), |item| {
                        item.bg(rgb(0xe6f7f1))
                    })
                    .child(svg().data(NAV_PROXY_ICON).size(px(18.)).text_color(rgb(
                        if active == Some(SettingsTab::Proxies) {
                            GREEN
                        } else {
                            INK
                        },
                    )))
                    .child("网络代理")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_tab = Some(SettingsTab::Proxies);
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .id("nav-tags")
                    .role(Role::Button)
                    .aria_label("标签")
                    .h(px(42.))
                    .px_3()
                    .gap_3()
                    .items_center()
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(rgb(if active == Some(SettingsTab::Tags) {
                        GREEN
                    } else {
                        INK
                    }))
                    .when(active == Some(SettingsTab::Tags), |item| {
                        item.bg(rgb(0xe6f7f1))
                    })
                    .child(svg().data(NAV_TAG_ICON).size(px(18.)).text_color(rgb(
                        if active == Some(SettingsTab::Tags) {
                            GREEN
                        } else {
                            INK
                        },
                    )))
                    .child("标签")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_tab = Some(SettingsTab::Tags);
                        this.load_tags(cx);
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .id("nav-settings")
                    .role(Role::Button)
                    .aria_label("全局设置")
                    .h(px(42.))
                    .w(px(42.))
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(rgb(
                        if matches!(active, Some(SettingsTab::General | SettingsTab::Data)) {
                            GREEN
                        } else {
                            INK
                        },
                    ))
                    .when(
                        matches!(active, Some(SettingsTab::General | SettingsTab::Data)),
                        |item| item.bg(rgb(0xe6f7f1)),
                    )
                    .child(Icon::new(IconName::Settings).size(px(18.)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_tab = Some(SettingsTab::General);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    pub(super) fn render_general_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w_full()
            .gap_5()
            .child(
                div()
                    .text_2xl()
                    .font_semibold()
                    .text_color(rgb(INK))
                    .child("常规"),
            )
            .child(
                v_flex()
                    .w_full()
                    .border_1()
                    .border_color(rgb(LINE))
                    .rounded_md()
                    .bg(rgb(0xffffff))
                    .child(
                        h_flex()
                            .p_4()
                            .justify_between()
                            .border_b_1()
                            .border_color(rgb(LINE))
                            .child(
                                div()
                                    .text_lg()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child("SOCKS5 网络代理"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(GREEN))
                                    .child(self.general_status.clone()),
                            ),
                    )
                    .child(
                        v_flex()
                            .p_5()
                            .gap_4()
                            .child(
                                h_flex()
                                    .gap_5()
                                    .child(settings_field("服务器", &self.general_host))
                                    .child(settings_field("端口", &self.general_port)),
                            )
                            .child(
                                h_flex()
                                    .gap_5()
                                    .child(settings_field("用户名", &self.general_username))
                                    .child(settings_field("密码", &self.general_password)),
                            )
                            .child(
                                h_flex()
                                    .gap_5()
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .w(px(90.))
                                                    .text_sm()
                                                    .text_color(rgb(INK))
                                                    .child("DNS 解析"),
                                            )
                                            .child(
                                                Button::new("general-dns")
                                                    .outline()
                                                    .small()
                                                    .label(if self.general.remote_dns {
                                                        "通过代理解析"
                                                    } else {
                                                        "本地解析"
                                                    })
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.general.remote_dns =
                                                            !this.general.remote_dns;
                                                        this.schedule_general_save(cx);
                                                    })),
                                            ),
                                    )
                                    .child(settings_field("连接超时（秒）", &self.general_timeout)),
                            )
                            .child(div().h(px(1.)).bg(rgb(LINE)))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_4()
                                    .child(
                                        Button::new("test-general-proxy")
                                            .outline()
                                            .small()
                                            .label(if self.general_testing {
                                                "检测中…"
                                            } else {
                                                "检测连接"
                                            })
                                            .disabled(self.general_testing)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.test_general_proxy(cx)
                                            })),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(
                                                if self.general_test.starts_with("连接失败") {
                                                    RED
                                                } else {
                                                    GREEN
                                                },
                                            ))
                                            .child(self.general_test.clone()),
                                    ),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .p_5()
                    .gap_4()
                    .border_1()
                    .border_color(rgb(LINE))
                    .rounded_md()
                    .bg(rgb(0xffffff))
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("数据目录"),
                    )
                    .child(storage_directory_row(
                        "浏览器文件工作目录",
                        self.paths.browser_root().to_path_buf(),
                        StorageKind::Browser,
                        self.storage_busy,
                        cx,
                    ))
                    .child(storage_directory_row(
                        "用户数据工作目录",
                        self.paths.profiles_root().to_path_buf(),
                        StorageKind::Profiles,
                        self.storage_busy,
                        cx,
                    ))
                    .when(!self.storage_status.is_empty(), |card| {
                        card.child(
                            div()
                                .text_sm()
                                .text_color(rgb(if self.storage_status.contains("失败") {
                                    RED
                                } else {
                                    GREEN
                                }))
                                .child(self.storage_status.clone()),
                        )
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_managed_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex();
        for (index, proxy) in self.managed.iter().enumerate() {
            let id = proxy.id.clone();
            let policy = match proxy.policy {
                ProxyPolicy::AllowParallel => "允许并发",
                ProxyPolicy::RejectNew => "拒绝新开",
                ProxyPolicy::ClosePrevious => "关闭已有",
            };
            let rotation = if proxy.ip_switch.as_ref().is_some_and(|rule| rule.on_start) {
                "开启"
            } else {
                "关闭"
            };
            let test = self
                .managed_tests
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "未检测".into());
            let testing = self.managed_testing.contains(&id);
            let test_id = id.clone();
            let edit_id = id.clone();
            rows = rows.child(
                h_flex()
                    .id(SharedString::from(format!("managed-row-{id}")))
                    .h(px(66.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .bg(rgb(if index % 2 == 0 { 0xffffff } else { 0xf8fbf9 }))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(180.))
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child(proxy.name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .text_ellipsis()
                                    .child(proxy.url.clone()),
                            ),
                    )
                    .child(
                        div()
                            .w(px(130.))
                            .text_xs()
                            .text_color(rgb(INK))
                            .child(policy),
                    )
                    .child(
                        div()
                            .w(px(105.))
                            .text_xs()
                            .text_color(rgb(if rotation == "开启" { GREEN } else { MUTED }))
                            .child(rotation),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(150.))
                            .text_xs()
                            .text_ellipsis()
                            .text_color(rgb(if test.starts_with("检测失败") {
                                RED
                            } else {
                                MUTED
                            }))
                            .child(test),
                    )
                    .child(
                        h_flex()
                            .w(px(120.))
                            .gap_2()
                            .child(
                                management_row_button(
                                    format!("test-managed-{test_id}"),
                                    if testing { "检测中" } else { "检测" },
                                )
                                .disabled(testing)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| this.test_managed(test_id.clone(), cx),
                                )),
                            )
                            .child(
                                management_row_button(format!("edit-managed-{edit_id}"), "编辑")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_managed_editor(Some(edit_id.clone()), window, cx)
                                    })),
                            ),
                    ),
            );
        }
        let table = v_flex()
            .flex_1()
            .w_full()
            .border_1()
            .border_color(rgb(LINE))
            .rounded_md()
            .overflow_hidden()
            .bg(rgb(0xffffff))
            .child(
                h_flex()
                    .h(px(39.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .bg(rgb(GREEN))
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(180.))
                            .text_xs()
                            .font_semibold()
                            .text_color(rgb(0xffffff))
                            .child("名称 / SOCKS5 地址"),
                    )
                    .child(
                        div()
                            .w(px(130.))
                            .text_xs()
                            .font_semibold()
                            .text_color(rgb(0xffffff))
                            .child("并发策略"),
                    )
                    .child(
                        div()
                            .w(px(105.))
                            .text_xs()
                            .font_semibold()
                            .text_color(rgb(0xffffff))
                            .child("切换 IP"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(150.))
                            .text_xs()
                            .font_semibold()
                            .text_color(rgb(0xffffff))
                            .child("出口检测"),
                    )
                    .child(
                        div()
                            .w(px(120.))
                            .text_xs()
                            .font_semibold()
                            .text_color(rgb(0xffffff))
                            .child("操作"),
                    ),
            )
            .child(if self.managed.is_empty() {
                div()
                    .h(px(120.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("还没有代理，点击右上角添加")
                    .into_any_element()
            } else {
                rows.into_any_element()
            })
            .into_any_element();

        v_flex()
            .w_full()
            .gap_4()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_2xl()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("代理管理"),
                    )
                    .child(
                        management_add_button("new-managed-proxy", "添加代理").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.open_managed_editor(None, window, cx)
                            }),
                        ),
                    ),
            )
            .when(!self.managed_status.is_empty(), |page| {
                page.child(
                    div()
                        .text_sm()
                        .text_color(rgb(if self.managed_status.contains("失败") {
                            RED
                        } else {
                            GREEN
                        }))
                        .child(self.managed_status.clone()),
                )
            })
            .child(table)
            .into_any_element()
    }

    pub(super) fn render_managed_editor(&self, home: Entity<Self>) -> AnyElement {
        let mut policy_buttons = h_flex().gap_2();
        for (policy, label) in [
            (ProxyPolicy::AllowParallel, "允许并发"),
            (ProxyPolicy::RejectNew, "拒绝新开"),
            (ProxyPolicy::ClosePrevious, "关闭已有"),
        ] {
            let home = home.clone();
            policy_buttons = policy_buttons.child(
                Button::new(format!("policy-{label}"))
                    .when(self.managed_policy == policy, |button| button.primary())
                    .when(self.managed_policy != policy, |button| button.outline())
                    .small()
                    .label(label)
                    .on_click(move |_, _, cx| {
                        let _ = home.update(cx, |this, cx| {
                            this.managed_policy = policy;
                            cx.notify();
                        });
                    }),
            );
        }
        let mut method_buttons = h_flex().gap_2();
        for (method, label) in [(SwitchMethod::Get, "GET"), (SwitchMethod::Post, "POST")] {
            let home = home.clone();
            method_buttons = method_buttons.child(
                Button::new(format!("switch-method-{label}"))
                    .when(self.managed_method == method, |button| button.primary())
                    .when(self.managed_method != method, |button| button.outline())
                    .small()
                    .label(label)
                    .on_click(move |_, _, cx| {
                        let _ = home.update(cx, |this, cx| {
                            this.managed_method = method;
                            cx.notify();
                        });
                    }),
            );
        }
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
                    .child(Input::new(&self.managed_name)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("SOCKS5 地址"),
                    )
                    .child(Input::new(&self.managed_url))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("格式：socks5://host:port"),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child("用户名（可选）"),
                            )
                            .child(Input::new(&self.managed_username)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child("密码（可选）"),
                            )
                            .child(Input::new(&self.managed_password)),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("并发策略"),
                    )
                    .child(policy_buttons),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("启动时切换 IP"),
                    )
                    .child(
                        Switch::new("managed-on-start")
                            .checked(self.managed_on_start)
                            .accessibility_label("启动时切换 IP")
                            .on_click({
                                let home = home.clone();
                                move |checked, _, cx| {
                                    let _ = home.update(cx, |this, cx| {
                                        this.managed_on_start = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            )
            .when(self.managed_on_start, |form| {
                form.child(
                    v_flex()
                        .gap_1()
                        .child(div().text_sm().text_color(rgb(INK)).child("切换地址"))
                        .child(Input::new(&self.managed_switch_url)),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(div().text_sm().text_color(rgb(INK)).child("HTTP 方法"))
                        .child(method_buttons),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(INK))
                                .child("切换后等待（秒）"),
                        )
                        .child(Input::new(&self.managed_wait)),
                )
            })
            .when(self.managed_selected.is_some(), |form| {
                form.child(
                    Button::new("delete-managed")
                        .ghost()
                        .small()
                        .label("删除代理")
                        .on_click({
                            let home = home.clone();
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                let _ = home
                                    .update(cx, |this, cx| this.confirm_remove_managed(window, cx));
                            }
                        }),
                )
            })
            .when(
                self.managed_status.contains("失败") || self.managed_status.contains("请"),
                |form| {
                    form.child(
                        div()
                            .text_sm()
                            .text_color(rgb(RED))
                            .child(self.managed_status.clone()),
                    )
                },
            )
            .into_any_element()
    }

    pub(super) fn render_data_settings(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let manager = self.browser_manager();
        let current = self.browser_versions.iter().find(|version| version.active);
        let current_label = current
            .map(|version| version.version.full_string())
            .unwrap_or_else(|| "尚未安装".into());
        let latest_label = self
            .browser_latest
            .as_ref()
            .map(|release| release.version.full_string())
            .unwrap_or_else(|| "尚未检查".into());
        let has_update = self.browser_latest.as_ref().is_some_and(|release| {
            current.is_none_or(|current| current.version != release.version)
        });
        let entity = cx.entity();
        let mut version_rows = v_flex().gap_2();
        for installed in &self.browser_versions {
            let version = installed.version.clone();
            let version_for_delete = version.clone();
            let key = version.full_string();
            let active = installed.active;
            version_rows = version_rows.child(
                h_flex()
                    .w_full()
                    .p_3()
                    .items_center()
                    .gap_4()
                    .rounded_md()
                    .bg(rgb(ROW_ALT))
                    .child(
                        div()
                            .w(px(160.))
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child(key.clone()),
                    )
                    .child(if active { "当前使用" } else { "已安装" })
                    .child(div().flex_1())
                    .child(
                        Button::new(format!("browser-activate-{key}"))
                            .outline()
                            .small()
                            .label("设为当前")
                            .disabled(active || self.browser_busy || !self.running.is_empty())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.activate_browser_version(version.clone(), cx);
                            })),
                    )
                    .child(
                        Button::new(format!("browser-delete-{key}"))
                            .outline()
                            .small()
                            .label("删除")
                            .disabled(active || self.browser_busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_delete_browser_version(
                                    version_for_delete.clone(),
                                    window,
                                    cx,
                                );
                            })),
                    ),
            );
        }
        if self.browser_versions.is_empty() {
            version_rows = version_rows.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("尚无已安装版本"),
            );
        }
        let progress = self.browser_progress.as_ref();
        let fraction = progress.and_then(|progress| {
            progress.total.map(|total| {
                if total == 0 {
                    0.
                } else {
                    (progress.received as f32 / total as f32).clamp(0., 1.)
                }
            })
        });
        let stage = progress
            .map(|progress| progress.stage)
            .unwrap_or(DownloadStage::Download);
        let progress_title = match stage {
            DownloadStage::Download => "正在下载",
            DownloadStage::Verify => "正在校验",
            DownloadStage::Install => "正在安装",
        };
        let progress_text = progress
            .map(|progress| {
                let received = progress.received as f64 / 1_048_576.;
                match progress.total {
                    Some(total) => format!("{received:.1} / {:.1} MB", total as f64 / 1_048_576.),
                    None => format!("已下载 {received:.1} MB"),
                }
            })
            .unwrap_or_default();
        v_flex()
            .w_full()
            .gap_4()
            .child(
                div()
                    .text_2xl()
                    .font_semibold()
                    .text_color(rgb(INK))
                    .child("数据与浏览器"),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .items_stretch()
                    .when(narrow, |row| row.flex_col())
                    .child(
                        v_flex()
                            .flex_1()
                            .p_5()
                            .gap_3()
                            .border_1()
                            .border_color(rgb(LINE))
                            .rounded_md()
                            .bg(rgb(0xffffff))
                            .child(
                                div()
                                    .font_semibold()
                                    .text_color(rgb(INK))
                                    .child("当前浏览器"),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_semibold()
                                            .text_color(rgb(INK))
                                            .child(current_label),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(if current.is_some() {
                                                GREEN
                                            } else {
                                                MUTED
                                            }))
                                            .child(if current.is_some() {
                                                "● 已安装"
                                            } else {
                                                "未安装"
                                            }),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("browser-check-update")
                                            .outline()
                                            .small()
                                            .label(if self.browser_checking {
                                                "检查中…"
                                            } else {
                                                "检查更新"
                                            })
                                            .disabled(self.browser_checking || self.browser_busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.check_browser_update(cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("browser-manage-versions")
                                            .outline()
                                            .small()
                                            .label(if self.browser_versions_expanded {
                                                "收起版本"
                                            } else {
                                                "管理版本"
                                            })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.browser_versions_expanded =
                                                    !this.browser_versions_expanded;
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .p_5()
                            .gap_3()
                            .border_1()
                            .border_color(rgb(LINE))
                            .rounded_md()
                            .bg(rgb(0xffffff))
                            .child(div().font_semibold().text_color(rgb(INK)).child("可用更新"))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_semibold()
                                            .text_color(rgb(INK))
                                            .child(latest_label),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("browser-download-install")
                                            .primary()
                                            .small()
                                            .label(if self.browser_busy {
                                                "安装中…"
                                            } else {
                                                "下载并安装"
                                            })
                                            .disabled(
                                                !has_update
                                                    || self.browser_busy
                                                    || self.browser_checking,
                                            )
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.start_browser_download(cx)
                                            })),
                                    ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .py_2()
                    .child(
                        Checkbox::new("browser-download-global-proxy")
                            .checked(self.browser_download_settings.use_global_proxy)
                            .disabled(self.browser_busy || self.browser_checking)
                            .accessibility_label("使用全局代理检查和下载浏览器")
                            .on_click(move |checked, _, cx| {
                                entity.update(cx, |this, cx| {
                                    let settings = BrowserDownloadSettings {
                                        use_global_proxy: *checked,
                                    };
                                    match settings.save(this.service.data_dir()) {
                                        Ok(()) => {
                                            this.browser_download_settings = settings;
                                            this.browser_latest = None;
                                            this.browser_status =
                                                "下载网络设置已自动保存，请重新检查更新".into();
                                        }
                                        Err(error) => {
                                            this.browser_status = format!("保存失败：{error}")
                                        }
                                    }
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("使用全局代理检查和下载"),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .p_4()
                    .gap_3()
                    .border_1()
                    .border_color(rgb(LINE))
                    .rounded_md()
                    .bg(rgb(0xffffff))
                    .child(
                        div()
                            .font_semibold()
                            .text_color(rgb(INK))
                            .child("已安装版本"),
                    )
                    .when(self.browser_versions_expanded, |container| {
                        container.child(version_rows)
                    }),
            )
            .when(self.browser_busy, |container| {
                container.child(
                    v_flex()
                        .w_full()
                        .p_4()
                        .gap_3()
                        .border_1()
                        .border_color(rgb(LINE))
                        .rounded_md()
                        .bg(rgb(0xffffff))
                        .child(
                            h_flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .font_semibold()
                                        .text_color(rgb(INK))
                                        .child(progress_title),
                                )
                                .child(div().flex_1())
                                .child(div().text_sm().text_color(rgb(MUTED)).child(progress_text))
                                .child(
                                    Button::new("browser-pause-download")
                                        .outline()
                                        .small()
                                        .label(
                                            if self
                                                .browser_control
                                                .as_ref()
                                                .is_some_and(DownloadControl::is_paused)
                                            {
                                                "继续"
                                            } else {
                                                "暂停"
                                            },
                                        )
                                        .disabled(stage != DownloadStage::Download)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(control) = &this.browser_control {
                                                control.pause(!control.is_paused());
                                            }
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("browser-cancel-download")
                                        .outline()
                                        .small()
                                        .label("取消")
                                        .disabled(stage != DownloadStage::Download)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(control) = &this.browser_control {
                                                control.cancel();
                                                this.browser_status = "正在取消下载…".into();
                                            }
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(
                            div().w_full().h(px(9.)).rounded_full().bg(rgb(LINE)).child(
                                div()
                                    .w(relative(fraction.unwrap_or(0.)))
                                    .h_full()
                                    .rounded_full()
                                    .bg(rgb(GREEN)),
                            ),
                        ),
                )
            })
            .when(!self.browser_status.is_empty(), |container| {
                container.child(
                    div()
                        .text_sm()
                        .text_color(rgb(if self.browser_status.contains("失败") {
                            RED
                        } else {
                            GREEN
                        }))
                        .child(self.browser_status.clone()),
                )
            })
            .child(
                v_flex()
                    .p_5()
                    .gap_3()
                    .border_1()
                    .border_color(rgb(LINE))
                    .rounded_md()
                    .bg(rgb(0xffffff))
                    .child(div().font_semibold().text_color(rgb(INK)).child("本地数据"))
                    .child(data_location(
                        "浏览器用户数据",
                        self.paths.profiles(),
                        false,
                        cx,
                    ))
                    .child(data_location(
                        "浏览器安装目录",
                        manager.active_path(),
                        false,
                        cx,
                    )),
            )
            .into_any_element()
    }

    pub(super) fn render_settings(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let content = match self.settings_tab.unwrap_or(SettingsTab::General) {
            SettingsTab::General => self.render_general_settings(cx),
            SettingsTab::Proxies => self.render_managed_settings(cx),
            SettingsTab::Tags => self.render_tag_settings(cx),
            SettingsTab::Data => {
                let width: f32 = window.bounds().size.width.into();
                self.render_data_settings(width < 1320., cx)
            }
        };
        v_flex()
            .flex_1()
            .h_full()
            .min_h_0()
            .when(
                matches!(
                    self.settings_tab,
                    Some(SettingsTab::General | SettingsTab::Data)
                ),
                |page| {
                    page.child(
                        h_flex()
                            .h(px(48.))
                            .px_6()
                            .gap_2()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(LINE))
                            .child(
                                Button::new("settings-general-tab")
                                    .small()
                                    .icon(IconName::Settings2)
                                    .label("常规")
                                    .when(
                                        self.settings_tab == Some(SettingsTab::General),
                                        |button| button.primary(),
                                    )
                                    .when(
                                        self.settings_tab != Some(SettingsTab::General),
                                        |button| button.ghost(),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings_tab = Some(SettingsTab::General);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("settings-data-tab")
                                    .small()
                                    .icon(IconName::HardDrive)
                                    .label("数据与浏览器")
                                    .when(self.settings_tab == Some(SettingsTab::Data), |button| {
                                        button.primary()
                                    })
                                    .when(self.settings_tab != Some(SettingsTab::Data), |button| {
                                        button.ghost()
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings_tab = Some(SettingsTab::Data);
                                        cx.notify();
                                    })),
                            ),
                    )
                },
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_6()
                    .child(content),
            )
            .into_any_element()
    }
}
