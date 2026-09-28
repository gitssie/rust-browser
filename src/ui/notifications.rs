use super::*;

struct AppStatusNotification;

impl BrowserHome {
    pub(super) fn render_pending_notice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(notice) = self.pending_notice.take() else {
            return;
        };
        self.status_generation = self.status_generation.wrapping_add(1);
        let generation = self.status_generation;
        window.defer(cx, move |window, cx| {
            let notification = notice
                .id::<AppStatusNotification>()
                .placement(Anchor::BottomRight);
            window.push_notification(notification, cx);
        });
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.status_generation == generation {
                    window.remove_notification::<AppStatusNotification>(cx);
                }
            });
        })
        .detach();
    }
}
