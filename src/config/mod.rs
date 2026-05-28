use crate::theme::ThemeConfig;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Deserialize)]
pub struct Config {
    pub layout: LayoutConfig,
    #[serde(default)]
    pub widgets: HashMap<String, WidgetDef>,
    #[serde(default)]
    pub theme: ThemeConfig,
}

#[derive(Deserialize)]
pub struct LayoutConfig {
    pub rows: Vec<RowConfig>,
}

#[derive(Deserialize)]
pub struct RowConfig {
    pub height: u16,
    pub slots: Vec<SlotConfig>,
}

#[derive(Deserialize)]
pub struct SlotConfig {
    pub width: u16,
    pub widget: String,
}

#[derive(Deserialize, Clone)]
pub struct WidgetDef {
    pub kind: String,
    #[serde(flatten)]
    pub params: HashMap<String, toml::Value>,
}

impl Config {
    pub fn load_or_default() -> Self {
        // 1. config.toml local (desarrollo)
        if let Ok(cfg) = Self::load(Path::new("config.toml")) {
            return cfg;
        }
        // 2. ~/.config/paneldecontrol/config.toml
        if let Some(dir) = crate::dirs::config_dir() {
            if let Ok(cfg) = Self::load(&dir.join("config.toml")) {
                return cfg;
            }
        }
        Self::builtin_default()
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }

    fn builtin_default() -> Self {
        // Hardcoded para cuando no existe ningún archivo de config.
        toml::from_str(
            r#"
            [[layout.rows]]
            height = 50
            slots = [
                { width = 50, widget = "widget-1" },
                { width = 50, widget = "widget-2" },
            ]
            [[layout.rows]]
            height = 50
            slots = [{ width = 100, widget = "widget-3" }]
        "#,
        )
        .expect("builtin config válida")
    }
}
