use crate::config::LayoutConfig;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
pub struct AppState {
    pub layout: Option<LayoutSnapshot>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LayoutSnapshot {
    pub rows: Vec<RowSnapshot>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RowSnapshot {
    pub height: u16,
    pub widths: Vec<u16>,
}

impl AppState {
    pub fn load() -> Self {
        crate::dirs::state_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = crate::dirs::state_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Ok(content) = toml::to_string(self) {
            std::fs::write(path, content).ok();
        }
    }

    pub fn from_layout(config: &LayoutConfig) -> Self {
        AppState {
            layout: Some(LayoutSnapshot {
                rows: config
                    .rows
                    .iter()
                    .map(|r| RowSnapshot {
                        height: r.height,
                        widths: r.slots.iter().map(|s| s.width).collect(),
                    })
                    .collect(),
            }),
        }
    }
}

impl LayoutSnapshot {
    // Aplica los porcentajes guardados al config cargado.
    // Si el número de filas/slots no coincide (config cambió), ignora silenciosamente.
    pub fn apply_to(&self, layout: &mut LayoutConfig) {
        for (snap_row, cfg_row) in self.rows.iter().zip(layout.rows.iter_mut()) {
            cfg_row.height = snap_row.height;
            // Solo aplicar anchos si el número de slots coincide exactamente.
            if snap_row.widths.len() == cfg_row.slots.len() {
                for (w, slot) in snap_row.widths.iter().zip(cfg_row.slots.iter_mut()) {
                    slot.width = *w;
                }
            }
        }
    }
}
