use std::collections::{BTreeMap, HashSet};

use egui_extras::{Column, TableBuilder, TableRow};

use crate::monitor::{self, Snapshot};
use crate::theme;
use crate::ui::widgets;

#[derive(Default, PartialEq, Copy, Clone)]
enum SortKey {
    Name,
    Pid,
    User,
    #[default]
    Cpu,
    Mem,
    Disk,
    Status,
}

#[derive(Default)]
pub struct State {
    sort: SortKey,
    descending: bool,
    selected_pid: Option<u32>,
    expanded: HashSet<String>,
    filter: String,
}

impl State {
    pub fn new() -> Self {
        Self {
            sort: SortKey::Cpu,
            descending: true,
            ..Default::default()
        }
    }
}

struct Group<'a> {
    name: String,
    procs: Vec<&'a monitor::processes::ProcInfo>,
    cpu_pct: f32,
    mem_bytes: u64,
    disk_bps: f64,
}

#[derive(Clone, Copy)]
enum Row<'a> {
    GroupHeader { g: &'a Group<'a>, expanded: bool },
    Single(&'a monitor::processes::ProcInfo),
    Child(&'a monitor::processes::ProcInfo),
}

pub fn show(ui: &mut egui::Ui, state: &mut State, snap: &Snapshot) {
    // Title row + selection actions on the right.
    ui.horizontal(|ui| {
        ui.heading("Processes");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(pid) = state.selected_pid {
                if ui.button("Force kill").clicked() {
                    let _ = monitor::processes::force_kill(pid);
                    state.selected_pid = None;
                }
                if ui.button("End task").clicked() {
                    let _ = monitor::processes::terminate(pid);
                    state.selected_pid = None;
                }
            } else {
                ui.add_enabled(false, egui::Button::new("End task"));
            }
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::singleline(&mut state.filter)
                    .hint_text("Filter by name or PID")
                    .desired_width(220.0),
            );
        });
    });

    ui.add_space(8.0);

    let filter = state.filter.trim().to_lowercase();
    let filter_active = !filter.is_empty();
    let matches = |p: &monitor::processes::ProcInfo| -> bool {
        if !filter_active {
            return true;
        }
        p.name.to_lowercase().contains(&filter) || p.pid.to_string().contains(&filter)
    };

    // Group processes by name. Singletons render as a regular row; groups with
    // multiple processes show a triangle + "(N)" count and aggregate values on
    // the header line. Expanding reveals the individual processes, sorted by
    // the same key.
    let mut by_name: BTreeMap<String, Vec<&monitor::processes::ProcInfo>> = BTreeMap::new();
    for p in &snap.processes {
        if matches(p) {
            by_name.entry(p.name.clone()).or_default().push(p);
        }
    }

    let mut groups: Vec<Group> = by_name
        .into_iter()
        .map(|(name, procs)| {
            let cpu_pct = procs.iter().map(|p| p.cpu_pct).sum();
            let mem_bytes = procs.iter().map(|p| p.mem_bytes).sum();
            let disk_bps = procs
                .iter()
                .map(|p| p.disk_read_bps + p.disk_write_bps)
                .sum();
            Group {
                name,
                procs,
                cpu_pct,
                mem_bytes,
                disk_bps,
            }
        })
        .collect();

    sort_groups(&mut groups, state.sort, state.descending);
    for g in &mut groups {
        sort_children(&mut g.procs, state.sort, state.descending);
    }

    let mut visible: Vec<Row> = Vec::with_capacity(snap.processes.len());
    for g in &groups {
        if g.procs.len() == 1 {
            visible.push(Row::Single(g.procs[0]));
        } else {
            let expanded = filter_active || state.expanded.contains(&g.name);
            visible.push(Row::GroupHeader { g, expanded });
            if expanded {
                for p in &g.procs {
                    visible.push(Row::Child(p));
                }
            }
        }
    }

    let total = snap.processes.len();
    let total_cpu_used: f32 = snap.system.cpu_total;
    let total_mem_used: f32 = snap.system.ram_used_pct;
    let selected_pid = state.selected_pid;

    // Deferred mutations: the rows closure can't easily reach `state` because
    // the table cell closures move captures around. Collect events here and
    // apply them after the table is done.
    let mut toggle_group: Option<String> = None;
    let mut click_pid: Option<u32> = None;

    egui::Frame::new()
        .fill(theme::PANEL_BG)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            let header_height = 26.0;
            let row_height = 22.0;

            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::initial(260.0).at_least(140.0).clip(true))
                .column(Column::initial(60.0).at_least(50.0))
                .column(Column::initial(100.0).at_least(70.0))
                .column(Column::initial(80.0).at_least(60.0))
                .column(Column::initial(100.0).at_least(80.0))
                .column(Column::initial(120.0).at_least(90.0))
                .column(Column::remainder().at_least(70.0))
                .header(header_height, |mut header| {
                    header.col(|ui| {
                        sortable_header(ui, "Name", SortKey::Name, state);
                    });
                    header.col(|ui| {
                        sortable_header(ui, "PID", SortKey::Pid, state);
                    });
                    header.col(|ui| {
                        sortable_header(ui, "User", SortKey::User, state);
                    });
                    header.col(|ui| {
                        sortable_header(
                            ui,
                            &format!("CPU  {:.0}%", total_cpu_used),
                            SortKey::Cpu,
                            state,
                        );
                    });
                    header.col(|ui| {
                        sortable_header(
                            ui,
                            &format!("Memory  {:.0}%", total_mem_used),
                            SortKey::Mem,
                            state,
                        );
                    });
                    header.col(|ui| {
                        sortable_header(ui, "Disk (R+W)", SortKey::Disk, state);
                    });
                    header.col(|ui| {
                        sortable_header(ui, "Status", SortKey::Status, state);
                    });
                })
                .body(|body| {
                    body.rows(row_height, visible.len(), |mut row| {
                        let idx = row.index();
                        match visible[idx] {
                            Row::GroupHeader { g, expanded } => {
                                render_group_header(&mut row, g, expanded, &mut toggle_group);
                            }
                            Row::Single(p) => {
                                row.set_selected(selected_pid == Some(p.pid));
                                render_proc_row(&mut row, p, false, &mut click_pid);
                            }
                            Row::Child(p) => {
                                row.set_selected(selected_pid == Some(p.pid));
                                render_proc_row(&mut row, p, true, &mut click_pid);
                            }
                        }
                    });
                });
        });

    if let Some(name) = toggle_group
        && !state.expanded.remove(&name)
    {
        state.expanded.insert(name);
    }
    if let Some(pid) = click_pid {
        state.selected_pid = Some(pid);
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{total} processes"))
                .color(theme::TEXT_DIM)
                .small(),
        );
        if !snap.ready {
            ui.label(
                egui::RichText::new("  · sampling…")
                    .color(theme::TEXT_DIM)
                    .small(),
            );
        }
    });
}

fn render_group_header(
    row: &mut TableRow<'_, '_>,
    g: &Group<'_>,
    expanded: bool,
    toggle: &mut Option<String>,
) {
    row.col(|ui| {
        let arrow = if expanded { "▼" } else { "▶" };
        let text = format!("{arrow}  {}  ({})", g.name, g.procs.len());
        let resp = ui.add(
            egui::Label::new(egui::RichText::new(text).strong())
                .truncate()
                .sense(egui::Sense::click()),
        );
        if resp.clicked() {
            *toggle = Some(g.name.clone());
        }
    });
    row.col(|_| {});
    row.col(|_| {});
    row.col(|ui| {
        ui.label(egui::RichText::new(format_pct(g.cpu_pct)).strong());
    });
    row.col(|ui| {
        ui.label(egui::RichText::new(widgets::format_bytes(g.mem_bytes)).strong());
    });
    row.col(|ui| {
        ui.label(egui::RichText::new(widgets::format_bps(g.disk_bps)).strong());
    });
    row.col(|_| {});
}

fn render_proc_row(
    row: &mut TableRow<'_, '_>,
    p: &monitor::processes::ProcInfo,
    indent: bool,
    click_pid: &mut Option<u32>,
) {
    row.col(|ui| {
        if indent {
            ui.add_space(20.0);
        }
        let resp = ui.add(
            egui::Label::new(&p.name)
                .truncate()
                .sense(egui::Sense::click()),
        );
        if resp.clicked() {
            *click_pid = Some(p.pid);
        }
        resp.on_hover_text(if p.cmd.is_empty() { &p.exe } else { &p.cmd });
    });
    row.col(|ui| {
        ui.label(p.pid.to_string());
    });
    row.col(|ui| {
        ui.label(&p.user);
    });
    row.col(|ui| {
        ui.label(format_pct(p.cpu_pct));
    });
    row.col(|ui| {
        ui.label(widgets::format_bytes(p.mem_bytes));
    });
    row.col(|ui| {
        let combined = p.disk_read_bps + p.disk_write_bps;
        let resp = ui.label(widgets::format_bps(combined));
        resp.on_hover_text(format!(
            "Read: {}\nWrite: {}",
            widgets::format_bps(p.disk_read_bps),
            widgets::format_bps(p.disk_write_bps),
        ));
    });
    row.col(|ui| {
        ui.label(egui::RichText::new(&p.status).color(status_color(&p.status)));
    });
}

fn sortable_header(ui: &mut egui::Ui, label: &str, key: SortKey, state: &mut State) {
    let is_active = state.sort == key;
    let up_color = if is_active && !state.descending {
        theme::TEXT
    } else {
        theme::TEXT_DIM
    };
    let down_color = if is_active && state.descending {
        theme::TEXT
    } else {
        theme::TEXT_DIM
    };

    let resp = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add(egui::Label::new(egui::RichText::new(label).strong()).selectable(false));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = -3.0;
                ui.add_space(2.0);
                ui.label(egui::RichText::new("▲").color(up_color).size(8.0));
                ui.label(egui::RichText::new("▼").color(down_color).size(8.0));
            });
        })
        .response
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);

    if resp.clicked() {
        if state.sort == key {
            state.descending = !state.descending;
        } else {
            state.sort = key;
            state.descending = matches!(key, SortKey::Cpu | SortKey::Mem | SortKey::Disk);
        }
    }
}

fn sort_groups(groups: &mut [Group], key: SortKey, desc: bool) {
    groups.sort_by(|a, b| {
        let ord = match key {
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Pid => a
                .procs
                .iter()
                .map(|p| p.pid)
                .min()
                .cmp(&b.procs.iter().map(|p| p.pid).min()),
            SortKey::User => a
                .procs
                .first()
                .map(|p| p.user.as_str())
                .cmp(&b.procs.first().map(|p| p.user.as_str())),
            SortKey::Cpu => a
                .cpu_pct
                .partial_cmp(&b.cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal),
            SortKey::Mem => a.mem_bytes.cmp(&b.mem_bytes),
            SortKey::Disk => a
                .disk_bps
                .partial_cmp(&b.disk_bps)
                .unwrap_or(std::cmp::Ordering::Equal),
            SortKey::Status => a
                .procs
                .first()
                .map(|p| p.status.as_str())
                .cmp(&b.procs.first().map(|p| p.status.as_str())),
        };
        if desc { ord.reverse() } else { ord }
    });
}

fn sort_children(rows: &mut [&monitor::processes::ProcInfo], key: SortKey, desc: bool) {
    rows.sort_by(|a, b| {
        let ord = match key {
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Pid => a.pid.cmp(&b.pid),
            SortKey::User => a.user.cmp(&b.user),
            SortKey::Cpu => a
                .cpu_pct
                .partial_cmp(&b.cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal),
            SortKey::Mem => a.mem_bytes.cmp(&b.mem_bytes),
            SortKey::Disk => {
                let ax = a.disk_read_bps + a.disk_write_bps;
                let bx = b.disk_read_bps + b.disk_write_bps;
                ax.partial_cmp(&bx).unwrap_or(std::cmp::Ordering::Equal)
            }
            SortKey::Status => a.status.cmp(&b.status),
        };
        if desc { ord.reverse() } else { ord }
    });
}

fn format_pct(v: f32) -> String {
    if v < 0.05 {
        "0%".into()
    } else if v < 10.0 {
        format!("{v:.1}%")
    } else {
        format!("{v:.0}%")
    }
}

fn status_color(s: &str) -> egui::Color32 {
    match s {
        "Run" | "Running" => theme::OK,
        "Sleep" | "Sleeping" => theme::TEXT_DIM,
        "Idle" => theme::TEXT_DIM,
        "Stop" | "Stopped" => theme::WARN,
        "Zombie" | "Dead" => theme::ERR,
        _ => theme::TEXT,
    }
}
