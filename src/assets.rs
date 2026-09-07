use gpui_kit::{AssetSource, Result, SharedString, assets::AllAssets};
use std::borrow::Cow;

const BRAND_ASSETS: &[(&str, &[u8])] = &[
    (
        "brand/queri-mark.svg",
        include_bytes!("../assets/brand/queri-mark.svg"),
    ),
    (
        "brand/queri-logo.svg",
        include_bytes!("../assets/brand/queri-logo.svg"),
    ),
    (
        "brand/postgresql.svg",
        include_bytes!("../assets/brand/postgresql.svg"),
    ),
    (
        "brand/sqlite.svg",
        include_bytes!("../assets/brand/sqlite.svg"),
    ),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = BRAND_ASSETS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = AllAssets.list(path)?;
        paths.extend(
            BRAND_ASSETS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::{Assets, BRAND_ASSETS};
    use gpui_kit::{AssetSource, assets::IconName};

    #[test]
    fn branding_and_toolkit_icons_share_the_asset_source() {
        let paths = Assets.list("brand/").expect("brand assets");
        assert_eq!(paths.len(), BRAND_ASSETS.len());
        for path in paths {
            let bytes = Assets
                .load(&path)
                .expect("load brand asset")
                .expect("asset");
            assert!(!bytes.is_empty());
        }
        let icon = IconName::Search.path();
        assert!(Assets.load(&icon).expect("toolkit icon").is_some());
        assert!(
            Assets
                .list("icons/")
                .expect("toolkit icons")
                .contains(&icon)
        );
        assert!(Assets.load("").expect("empty path").is_none());
    }
}
