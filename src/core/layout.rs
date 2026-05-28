use crate::config::LayoutConfig;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub struct LayoutSlot {
    pub widget_id: String,
    pub area: Rect,
    pub row_idx: usize,
    pub col_idx: usize,
}

pub struct LayoutManager {
    config: LayoutConfig,
    slots: Vec<LayoutSlot>,
    focused: usize,
}

impl LayoutManager {
    pub fn new(config: LayoutConfig) -> Self {
        Self {
            config,
            slots: Vec::new(),
            focused: 0,
        }
    }

    pub fn recalculate(&mut self, area: Rect) {
        self.slots.clear();

        let row_constraints: Vec<Constraint> = self
            .config
            .rows
            .iter()
            .map(|r| Constraint::Percentage(r.height))
            .collect();

        let row_areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(row_constraints)
            .split(area);

        for (row_idx, (row_cfg, &row_area)) in
            self.config.rows.iter().zip(row_areas.iter()).enumerate()
        {
            let col_constraints: Vec<Constraint> = row_cfg
                .slots
                .iter()
                .map(|s| Constraint::Percentage(s.width))
                .collect();

            let col_areas = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(col_constraints)
                .split(row_area);

            for (col_idx, (slot_cfg, &slot_area)) in
                row_cfg.slots.iter().zip(col_areas.iter()).enumerate()
            {
                self.slots.push(LayoutSlot {
                    widget_id: slot_cfg.widget.clone(),
                    area: slot_area,
                    row_idx,
                    col_idx,
                });
            }
        }

        if self.focused >= self.slots.len() && !self.slots.is_empty() {
            self.focused = self.slots.len() - 1;
        }
    }

    pub fn focus_next(&mut self) {
        if !self.slots.is_empty() {
            self.focused = (self.focused + 1) % self.slots.len();
        }
    }

    pub fn focus_prev(&mut self) {
        if !self.slots.is_empty() {
            self.focused = self.focused.checked_sub(1).unwrap_or(self.slots.len() - 1);
        }
    }

    // +/- cambia el ancho del slot enfocado compensando al vecino.
    pub fn resize_focused_width(&mut self, delta: i32) {
        let Some(slot) = self.slots.get(self.focused) else {
            return;
        };
        let (row_i, col_i) = (slot.row_idx, slot.col_idx);
        let row = &mut self.config.rows[row_i];
        let n = row.slots.len();
        if n < 2 {
            return;
        }

        let cur = row.slots[col_i].width as i32;
        let new = (cur + delta).clamp(10, 90) as u16;
        let diff = new as i32 - cur;
        if diff == 0 {
            return;
        }

        let neighbor = if col_i + 1 < n { col_i + 1 } else { col_i - 1 };
        let ncur = row.slots[neighbor].width as i32;
        row.slots[col_i].width = new;
        row.slots[neighbor].width = (ncur - diff).max(10) as u16;
    }

    // {/} cambia la altura de la fila del slot enfocado compensando a la vecina.
    pub fn resize_focused_height(&mut self, delta: i32) {
        let Some(slot) = self.slots.get(self.focused) else {
            return;
        };
        let row_i = slot.row_idx;
        let n = self.config.rows.len();
        if n < 2 {
            return;
        }

        let cur = self.config.rows[row_i].height as i32;
        let new = (cur + delta).clamp(10, 90) as u16;
        let diff = new as i32 - cur;
        if diff == 0 {
            return;
        }

        let neighbor = if row_i + 1 < n { row_i + 1 } else { row_i - 1 };
        let ncur = self.config.rows[neighbor].height as i32;
        self.config.rows[row_i].height = new;
        self.config.rows[neighbor].height = (ncur - diff).max(10) as u16;
    }

    pub fn slots(&self) -> &[LayoutSlot] {
        &self.slots
    }

    pub fn focused_id(&self) -> usize {
        self.focused
    }

    pub fn focus_widget_id(&mut self, id: &str) {
        if let Some(idx) = self.slots.iter().position(|s| s.widget_id == id) {
            self.focused = idx;
        }
    }

    pub fn export_config(&self) -> &LayoutConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LayoutConfig, RowConfig, SlotConfig};

    fn two_by_two() -> LayoutManager {
        let config = LayoutConfig {
            rows: vec![
                RowConfig {
                    height: 50,
                    slots: vec![
                        SlotConfig {
                            width: 50,
                            widget: "a".into(),
                        },
                        SlotConfig {
                            width: 50,
                            widget: "b".into(),
                        },
                    ],
                },
                RowConfig {
                    height: 50,
                    slots: vec![SlotConfig {
                        width: 100,
                        widget: "c".into(),
                    }],
                },
            ],
        };
        let mut lm = LayoutManager::new(config);
        lm.recalculate(Rect::new(0, 0, 100, 40));
        lm
    }

    #[test]
    fn resize_width_increases_slot() {
        let mut lm = two_by_two();
        lm.focused = 0;
        lm.resize_focused_width(10);
        assert_eq!(lm.config.rows[0].slots[0].width, 60);
        assert_eq!(lm.config.rows[0].slots[1].width, 40);
    }

    #[test]
    fn resize_width_clamps_at_min() {
        let mut lm = two_by_two();
        lm.focused = 0;
        lm.resize_focused_width(-50); // intentar < 10%
        assert!(lm.config.rows[0].slots[0].width >= 10);
    }

    #[test]
    fn resize_height_increases_row() {
        let mut lm = two_by_two();
        lm.focused = 0;
        lm.resize_focused_height(10);
        assert_eq!(lm.config.rows[0].height, 60);
        assert_eq!(lm.config.rows[1].height, 40);
    }

    #[test]
    fn single_slot_row_width_noop() {
        let mut lm = two_by_two();
        lm.focused = 2; // slot "c", fila con un solo slot
        let before = lm.config.rows[1].slots[0].width;
        lm.resize_focused_width(10);
        assert_eq!(lm.config.rows[1].slots[0].width, before);
    }

    #[test]
    fn focus_wraps() {
        let mut lm = two_by_two();
        lm.focused = 2;
        lm.focus_next();
        assert_eq!(lm.focused, 0);
    }
}
