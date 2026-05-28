use crate::core::event::InputEvent;
use crate::widgets::{
    CoreMsg, DataState, SistemaMetrics, Widget, WidgetAction, WidgetConfig, WidgetContext,
    WidgetId, WidgetMsg, WorkerContext,
};
use anyhow::Result;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
};
use std::time::{Duration, Instant};
use tokio::task::AbortHandle;

const DIM_AMBER: Color = Color::Rgb(0x80, 0x58, 0x00);
const BAR_W: usize = 20;

pub struct SistemaWidget {
    id: WidgetId,
    metrics: Option<SistemaMetrics>,
    last_fetch: Option<Instant>,
    worker: Option<AbortHandle>,
}

impl SistemaWidget {
    pub async fn init(config: WidgetConfig, _ctx: WidgetContext) -> Result<Box<dyn Widget>> {
        Ok(Box::new(Self {
            id: config.id,
            metrics: None,
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

            sys.refresh_cpu_usage();
            tokio::time::sleep(Duration::from_secs(1)).await;

            loop {
                sys.refresh_cpu_usage();
                sys.refresh_memory();
                let disks = Disks::new_with_refreshed_list();

                let mut disk_data: Vec<(String, u64, u64)> = Vec::new();
                for disk in &disks {
                    let mount = disk.mount_point().to_string_lossy().to_string();
                    if skip_mount(&mount) {
                        continue;
                    }
                    let total = disk.total_space();
                    if total < 1 << 30 {
                        continue;
                    }
                    let used = total.saturating_sub(disk.available_space());
                    disk_data.push((fmt_mount(&mount, 8), used, total));
                }

                let metrics = SistemaMetrics {
                    cpu_pct: sys.global_cpu_info().cpu_usage(),
                    mem_used: sys.used_memory(),
                    mem_total: sys.total_memory(),
                    swap_used: sys.used_swap(),
                    swap_total: sys.total_swap(),
                    disks: disk_data,
                };

                let _ = tx
                    .send(CoreMsg {
                        widget_id: widget_id.clone(),
                        msg: WidgetMsg::Sistema(metrics),
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
        if let WidgetMsg::Sistema(m) = msg {
            self.metrics = Some(m);
            self.last_fetch = Some(Instant::now());
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let Some(ref m) = self.metrics else {
            ratatui::widgets::Widget::render(
                Paragraph::new("Leyendo...").style(Style::default().fg(DIM_AMBER)),
                area,
                buf,
            );
            return;
        };

        let now = chrono::Local::now();
        let ts = format!("{}  //  {}", now.format("%H:%M:%S"), now.format("%d:%m:%y"));

        let mut lines: Vec<Line<'static>> = vec![
            Line::from(vec![Span::styled(ts, Style::default().fg(DIM_AMBER))]),
            Line::from(""),
        ];

        lines.push(bar_line("CPU ", m.cpu_pct, BAR_W, ""));

        let mem_pct = pct(m.mem_used, m.mem_total);
        lines.push(bar_line(
            "MEM ",
            mem_pct,
            BAR_W,
            &format!("{}/{}", fmt_b(m.mem_used), fmt_b(m.mem_total)),
        ));

        if m.swap_total > 0 {
            let swp_pct = pct(m.swap_used, m.swap_total);
            lines.push(bar_line(
                "SWP ",
                swp_pct,
                BAR_W,
                &format!("{}/{}", fmt_b(m.swap_used), fmt_b(m.swap_total)),
            ));
        }

        for (label, used, total) in &m.disks {
            let disk_pct = pct(*used, *total);
            lines.push(bar_line(
                &format!("{:<8}", label),
                disk_pct,
                BAR_W,
                &format!("{}/{}", fmt_b(*used), fmt_b(*total)),
            ));
        }

        ratatui::widgets::Widget::render(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
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

fn pct_color(pct: f32) -> Color {
    if pct < 60.0 {
        Color::Rgb(0x4a, 0xf6, 0x26) // verde
    } else if pct < 80.0 {
        Color::Rgb(0xff, 0xb0, 0x00) // amber
    } else {
        Color::Rgb(0xff, 0x55, 0x55) // rojo
    }
}

fn bar_line(label: &str, pct: f32, width: usize, suffix: &str) -> Line<'static> {
    let filled = ((pct / 100.0) * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    let color = pct_color(pct);

    Line::from(vec![
        Span::raw(label.to_string()),
        Span::raw("["),
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(empty), Style::default().fg(DIM_AMBER)),
        Span::raw("]"),
        Span::styled(format!(" {:5.1}%", pct), Style::default().fg(color)),
        Span::styled(format!("  {}", suffix), Style::default().fg(DIM_AMBER)),
    ])
}

fn pct(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        used as f32 / total as f32 * 100.0
    }
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
    fn pct_zero_total() {
        assert_eq!(pct(0, 0), 0.0);
    }

    #[test]
    fn pct_mitad() {
        assert!((pct(50, 100) - 50.0).abs() < 0.01);
    }

    #[test]
    fn pct_color_verde() {
        assert_eq!(pct_color(30.0), Color::Rgb(0x4a, 0xf6, 0x26));
    }

    #[test]
    fn pct_color_amber() {
        assert_eq!(pct_color(70.0), Color::Rgb(0xff, 0xb0, 0x00));
    }

    #[test]
    fn pct_color_rojo() {
        assert_eq!(pct_color(90.0), Color::Rgb(0xff, 0x55, 0x55));
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
