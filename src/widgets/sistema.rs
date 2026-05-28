use crate::core::event::InputEvent;
use crate::widgets::{
    CoreMsg, DataState, Widget, WidgetAction, WidgetConfig, WidgetContext, WidgetId, WidgetMsg,
    WorkerContext,
};
use anyhow::Result;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Paragraph, Wrap},
};
use std::time::{Duration, Instant};
use tokio::task::AbortHandle;

pub struct SistemaWidget {
    id: WidgetId,
    lines: Vec<String>,
    last_fetch: Option<Instant>,
    worker: Option<AbortHandle>,
}

impl SistemaWidget {
    pub async fn init(config: WidgetConfig, _ctx: WidgetContext) -> Result<Box<dyn Widget>> {
        Ok(Box::new(Self {
            id: config.id,
            lines: Vec::new(),
            last_fetch: None,
            worker: None,
        }))
    }
}

impl Widget for SistemaWidget {
    fn id(&self) -> &WidgetId {
        &self.id
    }
    fn kind(&self) -> &str {
        "sistema"
    }

    fn data_state(&self) -> DataState {
        match self.last_fetch {
            None => DataState::Loading,
            Some(t) => DataState::Fresh { fetched_at: t },
        }
    }

    fn start_background(&mut self, ctx: WorkerContext) {
        let tx = ctx.tx;
        let widget_id = ctx.widget_id;

        let handle = tokio::spawn(async move {
            use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, RefreshKind, System};

            let mut sys = System::new_with_specifics(
                RefreshKind::new()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything()),
            );

            // Primera lectura de CPU — necesaria para calcular el delta en la siguiente.
            sys.refresh_cpu_usage();
            tokio::time::sleep(Duration::from_secs(1)).await;

            loop {
                sys.refresh_cpu_usage();
                sys.refresh_memory();
                let disks = Disks::new_with_refreshed_list();

                let lines = build_lines(&sys, &disks);
                let _ = tx
                    .send(CoreMsg {
                        widget_id: widget_id.clone(),
                        msg: WidgetMsg::Lines(lines),
                    })
                    .await;

                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
        self.worker = Some(handle.abort_handle());
    }

    fn stop(&mut self) {
        if let Some(h) = self.worker.take() {
            h.abort();
        }
    }

    fn update(&mut self, msg: WidgetMsg) {
        if let WidgetMsg::Lines(lines) = msg {
            self.lines = lines;
            self.last_fetch = Some(Instant::now());
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let (text, style) = if self.lines.is_empty() {
            (
                "Leyendo...".to_string(),
                Style::default().fg(Color::DarkGray),
            )
        } else {
            (self.lines.join("\n"), Style::default())
        };
        ratatui::widgets::Widget::render(
            Paragraph::new(text).style(style).wrap(Wrap { trim: false }),
            area,
            buf,
        );
    }

    fn handle_input(&mut self, _ev: InputEvent) -> WidgetAction {
        WidgetAction::None
    }

    fn serialize_state(&self) -> toml::Value {
        toml::Value::Table(toml::map::Map::new())
    }
}

// ──────────────────────────────────────────────────────────────

fn build_lines(sys: &sysinfo::System, disks: &sysinfo::Disks) -> Vec<String> {
    const BAR: usize = 20;
    let mut lines = Vec::new();

    let cpu_pct = sys.global_cpu_info().cpu_usage();
    lines.push(format!("CPU  {} {:5.1}%", bar(cpu_pct, BAR), cpu_pct));

    let mem_used = sys.used_memory();
    let mem_total = sys.total_memory();
    let mem_pct = pct(mem_used, mem_total);
    lines.push(format!(
        "MEM  {} {}/{}",
        bar(mem_pct, BAR),
        fmt_b(mem_used),
        fmt_b(mem_total),
    ));

    let swp_used = sys.used_swap();
    let swp_total = sys.total_swap();
    if swp_total > 0 {
        let swp_pct = pct(swp_used, swp_total);
        lines.push(format!(
            "SWP  {} {}/{}",
            bar(swp_pct, BAR),
            fmt_b(swp_used),
            fmt_b(swp_total),
        ));
    }

    for disk in disks {
        let mount = disk.mount_point().to_string_lossy();
        if skip_mount(&mount) {
            continue;
        }
        let total = disk.total_space();
        let avail = disk.available_space();
        if total < 1 << 30 {
            continue;
        } // ignorar < 1 GiB
        let used = total.saturating_sub(avail);
        let label = fmt_mount(&mount, 8);
        lines.push(format!(
            "{} {} {}/{}",
            label,
            bar(pct(used, total), BAR),
            fmt_b(used),
            fmt_b(total),
        ));
    }

    lines
}

fn pct(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        used as f32 / total as f32 * 100.0
    }
}

fn bar(pct: f32, width: usize) -> String {
    let filled = ((pct / 100.0) * width as f32).round() as usize;
    let filled = filled.min(width);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(width - filled))
}

fn fmt_b(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1}G", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.0}M", b as f64 / (1u64 << 20) as f64),
        b => format!("{}K", b / 1024),
    }
}

fn fmt_mount(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        format!("{:<width$}", s, width = max)
    } else {
        format!("{}…", chars[..max - 1].iter().collect::<String>())
    }
}

fn skip_mount(m: &str) -> bool {
    let prefixes = ["/sys", "/proc", "/dev", "/run", "/snap", "/boot/efi"];
    prefixes
        .iter()
        .any(|p| m == *p || m.starts_with(&format!("{}/", p)))
}

// ──────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_vacio() {
        let b = bar(0.0, 10);
        assert_eq!(b, "[░░░░░░░░░░]");
    }

    #[test]
    fn bar_lleno() {
        let b = bar(100.0, 10);
        assert_eq!(b, "[██████████]");
    }

    #[test]
    fn bar_mitad() {
        let b = bar(50.0, 10);
        assert_eq!(b, "[█████░░░░░]");
    }

    #[test]
    fn fmt_b_gib() {
        assert_eq!(fmt_b(2 * (1 << 30)), "2.0G");
    }

    #[test]
    fn fmt_b_mib() {
        assert_eq!(fmt_b(512 * (1 << 20)), "512M");
    }

    #[test]
    fn skip_mount_proc() {
        assert!(skip_mount("/proc"));
        assert!(skip_mount("/proc/sys"));
        assert!(!skip_mount("/home"));
        assert!(!skip_mount("/"));
    }
}
