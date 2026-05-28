use ratatui::style::Color;
use ratatui::widgets::BorderType;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Default)]
pub struct ThemeConfig {
    pub accent: Option<String>,
    pub accent_focus: Option<String>,
    pub inactive: Option<String>,
    pub border_style: Option<String>,
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
            .unwrap_or(Color::Rgb(0xbd, 0x93, 0xf9));
        let accent_focused = cfg
            .accent_focus
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(Color::Rgb(0xff, 0x79, 0xc6));
        let inactive = cfg
            .inactive
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(Color::Rgb(0x44, 0x47, 0x5a));
        let border_style = cfg
            .border_style
            .as_deref()
            .and_then(parse_border_type)
            .unwrap_or(BorderType::Rounded);
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

    // Devuelve el accent del widget si está configurado, o el accent global.
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
        assert_eq!(parse_hex("#bd93f9"), Some(Color::Rgb(0xbd, 0x93, 0xf9)));
        assert_eq!(parse_hex("50fa7b"), Some(Color::Rgb(0x50, 0xfa, 0x7b)));
    }

    #[test]
    fn parse_hex_invalid() {
        assert_eq!(parse_hex("#gg0000"), None);
        assert_eq!(parse_hex("#fff"), None);
    }

    #[test]
    fn theme_widget_accent_fallback() {
        let theme = Theme::default();
        // Sin widget_accents configurados, devuelve el accent global.
        assert_eq!(theme.accent_for("cualquier-widget"), theme.accent);
    }
}
