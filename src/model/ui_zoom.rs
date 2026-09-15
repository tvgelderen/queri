use serde::{Deserialize, Serialize};

/// A supported interface zoom level, persisted as an integer percentage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "u16", into = "u16")]
pub struct UiZoom(u16);

impl Default for UiZoom {
    fn default() -> Self {
        Self(100)
    }
}

impl From<u16> for UiZoom {
    fn from(percent: u16) -> Self {
        if Self::LEVELS.contains(&percent) {
            Self(percent)
        } else {
            Self::default()
        }
    }
}

impl From<UiZoom> for u16 {
    fn from(zoom: UiZoom) -> Self {
        zoom.0
    }
}

impl UiZoom {
    pub const LEVELS: [u16; 8] = [80, 90, 100, 110, 125, 150, 175, 200];

    pub fn percent(self) -> u16 {
        self.0
    }

    pub fn factor(self) -> f32 {
        f32::from(self.0) / 100.
    }

    pub fn increase(self) -> Self {
        Self::LEVELS
            .iter()
            .find(|&&level| level > self.0)
            .copied()
            .map(Self)
            .unwrap_or(self)
    }

    pub fn decrease(self) -> Self {
        Self::LEVELS
            .iter()
            .rev()
            .find(|&&level| level < self.0)
            .copied()
            .map(Self)
            .unwrap_or(self)
    }
}

#[cfg(test)]
mod tests {
    use super::UiZoom;

    #[test]
    fn zoom_steps_stop_at_limits_and_retrace_every_level() {
        let mut zoom = UiZoom::from(80);
        assert_eq!(zoom.decrease(), zoom);
        for percent in UiZoom::LEVELS.into_iter().skip(1) {
            zoom = zoom.increase();
            assert_eq!(zoom.percent(), percent);
        }
        assert_eq!(zoom.increase(), zoom);
        for percent in UiZoom::LEVELS.into_iter().rev().skip(1) {
            zoom = zoom.decrease();
            assert_eq!(zoom.percent(), percent);
        }
    }

    #[test]
    fn unsupported_saved_percentages_fall_back_to_default() {
        for value in ["0", "99", "201", "65535"] {
            assert_eq!(
                serde_json::from_str::<UiZoom>(value).unwrap(),
                UiZoom::default()
            );
        }
    }
}
