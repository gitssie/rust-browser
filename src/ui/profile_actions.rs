use super::*;

impl BrowserHome {
    pub(super) fn show_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = Dialog::Create;
        if let Ok(managed) = ProxyCatalog::new(self.service.data_dir()).list() {
            self.managed = managed;
        }
        let options = profile_proxy_options(&self.global_proxy, &self.managed, None);
        self.form_proxy_select.update(cx, |select, cx| {
            select.set_items(options, window, cx);
            select.set_selected_value(&ProfileProxySelection::Global, window, cx);
        });
        self.form_os = host_profile_os();
        self.form_extra_tabs.clear();
        self.create_mode = CreateMode::Smart;
        self.geo_preview = None;
        self.geo_error.clear();
        self.geo_generation += 1;
        for input in [
            &self.form_name,
            &self.form_url,
            &self.form_country_code,
            &self.form_country,
            &self.form_region,
            &self.form_city,
            &self.form_timezone,
            &self.form_locale,
            &self.form_latitude,
            &self.form_longitude,
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.detect_geo(cx);
        cx.notify();
    }

    pub(super) fn show_edit(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.geo_generation += 1;
        if let Ok(managed) = ProxyCatalog::new(self.service.data_dir()).list() {
            self.managed = managed;
        }
        let Some(profile) = self.rows.iter().find(|row| row.id == id) else {
            return;
        };
        self.form_name.update(cx, |input, cx| {
            input.set_value(profile.name.as_deref().unwrap_or(""), window, cx)
        });
        self.form_url.update(cx, |input, cx| {
            input.set_value(
                profile.tabs.first().map(String::as_str).unwrap_or(""),
                window,
                cx,
            )
        });
        self.form_extra_tabs = profile
            .tabs
            .iter()
            .skip(1)
            .map(|url| {
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx).placeholder("https://example.com/");
                    input.set_value(url, window, cx);
                    input
                })
            })
            .collect();
        let proxy_choice = profile.proxy.clone();
        let selected = match &proxy_choice {
            ProxyChoice::Global => ProfileProxySelection::Global,
            ProxyChoice::Direct => ProfileProxySelection::Direct,
            ProxyChoice::Custom(url) => self
                .managed
                .iter()
                .find(|proxy| &proxy.url == url)
                .map(|proxy| ProfileProxySelection::Managed(proxy.id.clone()))
                .unwrap_or_else(|| ProfileProxySelection::ExistingCustom(url.clone())),
        };
        let options = profile_proxy_options(&self.global_proxy, &self.managed, Some(&proxy_choice));
        self.form_proxy_select.update(cx, |select, cx| {
            select.set_items(options, window, cx);
            select.set_selected_value(&selected, window, cx);
        });
        self.dialog = Dialog::Edit(id);
        cx.notify();
    }

    pub(super) fn selected_form_proxy(&self, cx: &Context<Self>) -> Result<ProxyChoice> {
        match self.form_proxy_select.read(cx).selected_value() {
            Some(selection) => profile_proxy_choice(selection, &self.managed),
            None => Err(anyhow!("请选择网络连接方式")),
        }
    }

    pub(super) fn detect_geo(&mut self, cx: &mut Context<Self>) {
        let proxy = match self.selected_form_proxy(cx) {
            Ok(ProxyChoice::Global) => Ok(Some(self.global_proxy.clone())),
            Ok(ProxyChoice::Direct) => Ok(None),
            Ok(ProxyChoice::Custom(url)) => ProxyCatalog::new(self.service.data_dir())
                .resolve_url(&url)
                .map(Some),
            Err(error) => Err(error),
        };
        let proxy = match proxy {
            Ok(proxy) => proxy,
            Err(error) => {
                self.geo_loading = false;
                self.geo_preview = None;
                self.geo_error = format!("所选代理不可用：{error}");
                cx.notify();
                return;
            }
        };
        self.geo_generation += 1;
        let generation = self.geo_generation;
        self.geo_loading = true;
        self.geo_preview = None;
        self.geo_error.clear();
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    tokio.block_on(async {
                        if let Some(proxy) = &proxy {
                            proxy.check().await?;
                        }
                        ProfileGeo::lookup_with_proxy(proxy.as_ref()).await
                    })
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if generation != this.geo_generation || !matches!(this.dialog, Dialog::Create) {
                    return;
                }
                this.geo_loading = false;
                match result {
                    Ok(geo) => {
                        this.fill_geo_fields(&geo, window, cx);
                        this.geo_preview = Some(geo);
                    }
                    Err(error) => this.geo_error = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn fill_geo_fields(
        &mut self,
        geo: &ProfileGeo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, value) in [
            (&self.form_country_code, geo.country_code.clone()),
            (&self.form_country, geo.country.clone()),
            (&self.form_region, geo.region.clone().unwrap_or_default()),
            (&self.form_city, geo.city.clone().unwrap_or_default()),
            (&self.form_timezone, geo.timezone.clone()),
            (&self.form_locale, geo.locale.clone()),
            (&self.form_latitude, geo.latitude.to_string()),
            (&self.form_longitude, geo.longitude.to_string()),
        ] {
            input.update(cx, |input, cx| input.set_value(&value, window, cx));
        }
    }

    pub(super) fn custom_geo(&self, cx: &Context<Self>) -> Result<ProfileGeoInput> {
        let value = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        Ok(ProfileGeoInput {
            country_code: value(&self.form_country_code),
            country: value(&self.form_country),
            region: Some(value(&self.form_region)),
            city: Some(value(&self.form_city)),
            timezone: value(&self.form_timezone),
            locale: value(&self.form_locale),
            latitude: value(&self.form_latitude)
                .parse()
                .map_err(|_| anyhow!("纬度必须是数字"))?,
            longitude: value(&self.form_longitude)
                .parse()
                .map_err(|_| anyhow!("经度必须是数字"))?,
        })
    }

    pub(super) fn submit_form(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let name = self.form_name.read(cx).value().trim().to_string();
        let url = self.form_url.read(cx).value().trim().to_string();
        let choice = match self.selected_form_proxy(cx) {
            Ok(choice) => choice,
            Err(error) => {
                self.geo_error = error.to_string();
                cx.notify();
                return;
            }
        };
        let tabs = std::iter::once(url)
            .chain(
                self.form_extra_tabs
                    .iter()
                    .map(|input| input.read(cx).value().trim().to_string()),
            )
            .filter(|url| !url.is_empty())
            .collect::<Vec<_>>();
        let service = self.service.clone();
        let tokio = self.tokio.clone();
        let dialog = self.dialog.clone();
        let is_create = matches!(dialog, Dialog::Create);
        let os = self.form_os;
        let screen = profile_screen(window, cx);
        let geo = if matches!(dialog, Dialog::Create) && self.create_mode == CreateMode::Custom {
            match self.custom_geo(cx) {
                Ok(geo) => Some(geo),
                Err(error) => {
                    self.geo_error = error.to_string();
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        self.geo_error.clear();
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    tokio.block_on(async {
                        match dialog {
                            Dialog::Create => {
                                service
                                    .create(
                                        CreateProfile::new(
                                            if name.is_empty() { None } else { Some(name) },
                                            os,
                                            tabs,
                                            choice,
                                            geo,
                                        )
                                        .with_screen(screen),
                                    )
                                    .await
                            }
                            Dialog::Edit(id) => {
                                service
                                    .update(
                                        &id,
                                        UpdateProfile {
                                            name: Some(name),
                                            tabs: Some(tabs),
                                            proxy: Some(choice),
                                            tags: None,
                                        },
                                    )
                                    .await
                            }
                            _ => unreachable!(),
                        }
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(profile) => {
                        this.pending_notice =
                            Some(Notification::info(format!("已保存 {}", profile.id)));
                        this.dialog = Dialog::None;
                        this.reload(cx);
                        this.load_tags(cx);
                    }
                    Err(error) if is_create => this.geo_error = error.to_string(),
                    Err(error) => {
                        this.pending_notice = Some(Notification::error(error.to_string()))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn delete_profile(&mut self, id: String, cx: &mut Context<Self>) {
        let service = self.service.clone();
        let tokio = self.tokio.clone();
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { tokio.block_on(service.delete(&id)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.dialog = Dialog::None;
                match result {
                    Ok(()) => {
                        this.pending_notice = Some(Notification::info("配置已删除"));
                        this.reload(cx);
                        this.load_tags(cx);
                    }
                    Err(error) => {
                        this.pending_notice = Some(Notification::error(error.to_string()))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_profile_tags(
        &mut self,
        id: String,
        tags: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let service = self.service.clone();
        let tokio = self.tokio.clone();
        let update_id = id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    tokio.block_on(service.update(
                        &update_id,
                        UpdateProfile {
                            tags: Some(tags),
                            ..Default::default()
                        },
                    ))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(profile) => {
                        if this.filter_tags.is_empty() {
                            if let Some(row) = this.rows.iter_mut().find(|row| row.id == id) {
                                *row = profile;
                            }
                        } else {
                            this.reload(cx);
                        }
                        this.load_tags(cx);
                    }
                    Err(error) => {
                        this.pending_notice = Some(Notification::error(error.to_string()))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn toggle_tag(&mut self, id: String, tag: String, cx: &mut Context<Self>) {
        let Some(profile) = self.rows.iter().find(|profile| profile.id == id) else {
            return;
        };
        let mut tags = profile.tags.clone();
        if let Some(index) = tags
            .iter()
            .position(|existing| existing.eq_ignore_ascii_case(&tag))
        {
            tags.remove(index);
        } else {
            tags.push(tag);
        }
        self.set_profile_tags(id, tags, cx);
    }
}
