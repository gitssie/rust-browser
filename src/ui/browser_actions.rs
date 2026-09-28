use super::*;

impl BrowserHome {
    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.rows.clear();
        self.page = 1;
        self.total = 0;
        self.loading = false;
        self.has_more = true;
        self.request_page(cx);
    }

    pub(super) fn request_page(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.has_more {
            return;
        }
        self.loading = true;
        let generation = self.generation;
        let page = self.page;
        let service = self.service.clone();
        let tokio = self.tokio.clone();
        let query = ProfileQuery {
            search: Some(self.search.read(cx).value().to_string()),
            page,
            page_size: 100,
        };
        let filter = ProfileListFilter {
            proxy_mode: None,
            proxy: self.proxy_filter.clone(),
            tags: self.filter_tags.clone(),
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { tokio.block_on(service.list_filtered(query, filter)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(page_result) => {
                        this.total = page_result.total;
                        this.rows.extend(page_result.items);
                        this.page += 1;
                        this.has_more = this.rows.len() < this.total;
                    }
                    Err(error) => {
                        this.has_more = false;
                        this.pending_notice = Some(Notification::error(error.to_string()));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_running(&mut self, cx: &mut Context<Self>) {
        let service = self.service.clone();
        let runtime = self.runtime.clone();
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    tokio.block_on(async {
                        let ids = service.list_ids().await?;
                        let mut running = HashSet::new();
                        for id in ids {
                            if runtime.is_running(&id).await {
                                running.insert(id);
                            }
                        }
                        Ok::<_, rust_browser::profiles::ProfileError>(running)
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let launching = this.launch.as_ref().map(|launch| launch.id.as_str());
                this.spawned.retain(|id, child| {
                    Some(id.as_str()) == launching || child.try_wait().ok().flatten().is_none()
                });
                if let Ok(running) = result
                    && this.running != running
                {
                    this.running = running;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn start_polling(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(1500))
                    .await;
                if this
                    .update(cx, |this, cx| this.refresh_running(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn open_browser(&mut self, id: String, window: &Window, cx: &mut Context<Self>) {
        if self.browser_busy {
            self.pending_notice = Some(Notification::info("浏览器版本正在安装，请稍后打开"));
            cx.notify();
            return;
        }
        if self.running.contains(&id) || self.spawned.contains_key(&id) || self.launch.is_some() {
            return;
        }
        let Some(profile) = self.rows.iter().find(|profile| profile.id == id) else {
            return;
        };
        let name = profile.name.clone().unwrap_or_else(|| profile.id.clone());
        let location = format!(
            "{} · {}",
            profile.saved_geo.country,
            profile.saved_geo.city.as_deref().unwrap_or("-")
        );
        let screen = profile_screen(window, cx);
        let result = (|| -> Result<(Child, PathBuf)> {
            let exe = std::env::current_exe()?;
            let log_dir = self.paths.logs();
            fs::create_dir_all(&log_dir)?;
            let progress_file = self.paths.launch_progress(&id, rand::random::<u64>());
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&progress_file)?;
            let log = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.paths.profile_log(&id))?;
            let stdout_log = log.try_clone()?;
            let mut command = ProcessCommand::new(exe);
            command
                .arg("--data-dir")
                .arg(self.service.data_dir())
                .arg("open")
                .arg(&id)
                .arg("--progress-file")
                .arg(&progress_file)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            // The native manager uses the saved general proxy, including its
            // credentials, even if the manager inherited a CLI proxy override.
            command.env_remove("RUST_BROWSER_PROXY");
            if let Some(screen) = screen {
                command.env(
                    "RUST_BROWSER_DISPLAY_SCREEN",
                    serde_json::to_string(&screen)?,
                );
            }
            #[cfg(unix)]
            command.process_group(0);
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    let _ = fs::remove_file(&progress_file);
                    return Err(error).context("start browser process");
                }
            };
            if let Some(stdout) = child.stdout.take() {
                mirror_launch_output(stdout, stdout_log, false);
            }
            if let Some(stderr) = child.stderr.take() {
                mirror_launch_output(stderr, log, true);
            }
            Ok((child, progress_file))
        })();
        match result {
            Ok((child, progress_file)) => {
                self.spawned.insert(id.clone(), child);
                self.launch = Some(LaunchUi {
                    id: id.clone(),
                    name,
                    location,
                    progress_file,
                    events_seen: 0,
                    stage: None,
                    close_count: None,
                    switch_ip: false,
                    error: None,
                    visible: true,
                });
                self.dialog = Dialog::None;
                self.start_launch_poll(cx);
            }
            Err(error) => self.pending_notice = Some(Notification::error(error.to_string())),
        }
        cx.notify();
    }

    pub(super) fn start_launch_poll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let Ok(keep_polling) = this.update(cx, |this, cx| this.poll_launch(cx)) else {
                    break;
                };
                if !keep_polling {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn poll_launch(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(launch) = self.launch.as_mut() else {
            return false;
        };
        let events = match read_events(&launch.progress_file) {
            Ok(events) => events,
            Err(error) => {
                launch.error = Some(format!("读取启动进度失败：{error}"));
                launch.visible = true;
                cx.notify();
                return false;
            }
        };
        let mut terminal = false;
        let mut ready = false;
        for event in events.iter().skip(launch.events_seen) {
            terminal = launch.apply(event);
            ready = matches!(event, LaunchEvent::Ready);
            if terminal {
                break;
            }
        }
        launch.events_seen = events.len();
        let id = launch.id.clone();
        let name = launch.name.clone();
        let path = launch.progress_file.clone();
        if ready {
            self.launch = None;
            self.pending_notice = Some(Notification::info(format!("已打开 {name}")));
            let _ = fs::remove_file(path);
            self.refresh_running(cx);
            self.reload(cx);
            cx.notify();
            return false;
        }
        if terminal {
            self.pending_notice = Some(Notification::error(format!("{name} 打开失败")));
            let _ = fs::remove_file(path);
            cx.notify();
            return false;
        }
        if let Some(child) = self.spawned.get_mut(&id)
            && let Ok(Some(status)) = child.try_wait()
        {
            launch.error = Some(format!(
                "启动进程已退出（{status}），请查看 {}",
                self.paths.profile_log(&id).display()
            ));
            launch.visible = true;
            self.pending_notice = Some(Notification::error(format!("{name} 打开失败")));
            let _ = fs::remove_file(path);
            cx.notify();
            return false;
        }
        cx.notify();
        true
    }

    pub(super) fn cancel_launch(&mut self, cx: &mut Context<Self>) {
        let Some(launch) = self.launch.take() else {
            return;
        };
        if let Some(mut child) = self.spawned.remove(&launch.id) {
            if child.try_wait().ok().flatten().is_none() {
                #[cfg(unix)]
                unsafe {
                    libc::killpg(child.id() as i32, libc::SIGKILL);
                }
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        let _ = fs::remove_file(launch.progress_file);
        self.pending_notice = Some(if launch.error.is_some() {
            Notification::error(format!("{} 打开失败", launch.name))
        } else {
            Notification::info(format!("已取消打开 {}", launch.name))
        });
        self.refresh_running(cx);
        cx.notify();
    }

    pub(super) fn close_browsers(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let runtime = self.runtime.clone();
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    tokio.block_on(async {
                        let mut errors = Vec::new();
                        for id in ids {
                            if let Err(error) = runtime.close(&id).await {
                                errors.push(format!("{id}: {error}"));
                            }
                        }
                        errors
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.pending_notice = Some(if result.is_empty() {
                    Notification::info("已请求关闭浏览器")
                } else {
                    Notification::error(result.join("; "))
                });
                this.refresh_running(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
