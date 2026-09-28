use super::*;
use std::sync::OnceLock;

pub(super) fn app_logo() -> Arc<Image> {
    static LOGO: OnceLock<Arc<Image>> = OnceLock::new();
    LOGO.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../assets/cazer-logo-ui.png").to_vec(),
        ))
    })
    .clone()
}
