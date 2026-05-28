use ratatui::style::Color;
use ratatui::widgets::BorderType;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Default)]
pub struct ThemeConfig {
    pub accent: Option<String>,
    #[serde(rename = "accent_focus")]
    pub accent_focus: Option<String>,
    pub inactive: Option<String>,
    #[serde(rename = "border_type")]
    pub border_type: Option<String>,
    pub widget_accents: Option<HashMap<String, String>>,
}

pub struct Theme {
    pub accent: Color,
    pub accent_focused: Color,
    pub inactive: Color,
    pub border_style: BorderType,
    pub widget_accents: HashMap<String, Color>,
}

impl Theme {
    pub fn from_config(cfg: &ThemeConfig) -> Self {
        let accent = cfg
            .accent
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(Color::Rgb(0xff, 0xb0, 0x00)); // amber
        let accent_focused = cfg
            .accent_focus
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(Color::Rgb(0x4a, 0xf6, 0x26)); // fósforo verde
        let inactive = cfg
            .inactive
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(Color::Rgb(0x3a, 0x20, 0x00)); // amber muy oscuro
        let border_style = cfg
            .border_type
            .as_deref()
            .and_then(parse_border_type)
            .unwrap_or(BorderType::Plain);
        let widget_accents = cfg
            .widget_accents
            .as_ref()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| parse_hex(v).map(|c| (k.clone(), c)))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            accent,
            accent_focused,
            inactive,
            border_style,
            widget_accents,
        }
    }

    pub fn accent_for(&self, widget_id: &str) -> Color {
        self.widget_accents
            .get(widget_id)
            .copied()
            .unwrap_or(self.accent)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::from_config(&ThemeConfig::default())
    }
}

fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

fn parse_border_type(s: &str) -> Option<BorderType> {
    match s {
        "plain" | "single" => Some(BorderType::Plain),
        "rounded" => Some(BorderType::Rounded),
        "double" => Some(BorderType::Double),
        "thick" => Some(BorderType::Thick),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_valid() {
        assert_eq!(parse_hex("#ffb000"), Some(Color::Rgb(0xff, 0xb0, 0x00)));
        assert_eq!(parse_hex("4af626"), Some(Color::Rgb(0x4a, 0xf6, 0x26)));
    }

    #[test]
    fn parse_hex_invalid() {
        assert_eq!(parse_hex("#gg0000"), None);
        assert_eq!(parse_hex("#fff"), None);
    }

    #[test]
    fn defaults_son_crt_amber() {
        let theme = Theme::default();
        assert_eq!(theme.accent, Color::Rgb(0xff, 0xb0, 0x00));
        assert_eq!(theme.accent_focused, Color::Rgb(0x4a, 0xf6, 0x26));
        assert_eq!(theme.border_style, BorderType::Plain);
    }

    #[test]
    fn theme_widget_accent_fallback() {
        let theme = Theme::default();
        assert_eq!(theme.accent_for("cualquier-widget"), theme.accent);
    }
}
