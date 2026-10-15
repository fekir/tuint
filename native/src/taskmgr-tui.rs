use std::{collections::HashSet, fmt::Write, rc::Rc};

#[allow(dead_code)]
mod common;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};

use crate::common::process::ProcessInfo;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

static HELP_TEXT: &str = //
    "Task Manager TUI

A TUI program for listing and stopping running tasks

Command-line parameters

 --help                   - show this dialog
 --version                - print version

TUI options

Arrow Keys/Mouse Wheel    - move up and down
Home/End                  - go to start/end of list
Q / F10 / Alt-0 / Ctrl+C  - close program
H / ? / F1 / Alt-1        - show this dialog
R / F5 / Ctrl+R / Alt-5   - refresh and redraw TUI
F9                        - kill process
F / Ctrl+F                - filter tasks
Shift+Z                   - toggle automatic refresh
P                         - toggle path
+ / -                     - collapse / expand tree";

struct List {
    alltasks: Vec<ProcessInfo>,
    tasks: Vec<usize>,
    selected: usize,
    top: usize,
    visible_height: usize,
    filter: String,
    collapsed: Vec<u32>, // sorted, pid
}

fn main() -> Result<()> {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--help" | "-h" | "?" | "/?" => {
                println!("{}", HELP_TEXT);
                return Ok(());
            }
            "--version" => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => {
                return Err(format!("Unrecognized command-line parameter: {}", other).into());
            }
        }
    }

    enable_raw_mode()?;

    crossterm::execute!(
        std::io::stdout(),
        crossterm::cursor::Hide,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    let res = run_tui();

    crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen,
        Clear(ClearType::All),
    )?;
    disable_raw_mode()?;
    return res;
}

fn run_tui() -> Result<()> {
    let mut fullpath = false;
    let mut paused = false;
    let mut needs_redraw = true;
    let mut list = List::new();
    list.reload(fullpath)?;

    let (mut last_width, mut last_height) =
        crossterm::terminal::size().map(|(w, h)| (w as usize, h as usize))?;

    let mut old = common::ui::ScreenBuffer::new();
    let mut new = common::ui::ScreenBuffer::new();
    common::ui::resize_buffers(&mut old, &mut new, last_width, last_height)?;
    let mut statusbar = String::new();
    let mut headerbar = String::new();

    loop {
        if needs_redraw {
            if last_width != new.width || last_height != new.height {
                last_width = new.width;
                last_height = new.height;
            }

            create_headerbar(&new, &list, &mut headerbar);
            list.visible_height = old.height.saturating_sub(3 + headerbar.lines().count()) as usize;
            list.top = list.top.min(list.selected);
            list.top = list.top.max(
                list.selected
                    .saturating_sub(list.visible_height.saturating_sub(1)),
            );

            if paused && statusbar.is_empty() {
                statusbar.push_str("PAUSED");
            }
            draw_ui(&mut new, &list, &statusbar, &headerbar);
            common::ui::print_diff(&mut old, &mut new)?;
            needs_redraw = false;
            statusbar.clear();
        }

        if !event::poll(std::time::Duration::from_secs(1))? {
            if !paused {
                list.reload(fullpath)?;
                needs_redraw = true;
            }
        } else {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    needs_redraw = true;
                    match (key.code, key.modifiers) {
                        (KeyCode::Char('q'), _)
                        | (KeyCode::Esc, _)
                        | (KeyCode::F(10), _)
                        | (KeyCode::Char('0'), KeyModifiers::ALT)
                        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break (),
                        (KeyCode::Char('?'), _)
                        | (KeyCode::Char('h'), _)
                        | (KeyCode::F(1), _)
                        | (KeyCode::Char('1'), KeyModifiers::ALT) => {
                            common::ui::info_dialog(&mut old, &mut new, HELP_TEXT)?;
                        }
                        (KeyCode::F(5), KeyModifiers::NONE)
                        | (KeyCode::Char('r'), KeyModifiers::NONE)
                        | (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                            // first move goto to function
                            let width = new.width;
                            let height = new.height;
                            common::ui::resize_buffers(&mut old, &mut new, width, height)?;
                            let start = std::time::Instant::now();
                            match list.reload(fullpath) {
                                Ok(v) => {
                                    write!(
                                        &mut statusbar,
                                        "Loaded {} tasks in {:.2?}",
                                        v,
                                        start.elapsed()
                                    )?;
                                }
                                Err(e) => {
                                    write!(&mut statusbar, "Error loading task: {}", e)?;
                                }
                            }
                        }
                        (KeyCode::F(9), KeyModifiers::NONE)
                        | (KeyCode::Delete, KeyModifiers::NONE) => {
                            if let Some(process) = list.get() {
                                if !process.subsystem.is_empty() {
                                    match common::ui::choose_dialog(
                                        &mut old,
                                        &mut new,
                                        &format!("Kill {} ({})?", process.name, process.pid),
                                        &["yes", "no"],
                                    ) {
                                        Ok(Some(0)) => {
                                            let result =
                                                if process.subsystem.as_ref() == "Windows" {
                                                    common::process::force_kill(process)
                                                } else {
                                                    common::wslprocess::kill_wsl_process(process)
                                                };

                                            if let Err(e) = result {
                                                write!(
                                                    &mut statusbar,
                                                    "Failed to kill {}: {e}",
                                                    process.pid
                                                )?;
                                            }
                                            list.reload(fullpath)?;
                                            needs_redraw = true;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }

                        (KeyCode::Down, KeyModifiers::NONE) => {
                            list.selected =
                                (list.selected + 1).min(list.tasks.len().saturating_sub(1));
                        }
                        (KeyCode::Up, KeyModifiers::NONE) => {
                            list.selected = list.selected.saturating_sub(1);
                        }

                        (KeyCode::PageDown, KeyModifiers::NONE) => {
                            list.selected = (list.selected + list.visible_height)
                                .min(list.tasks.len().saturating_sub(1));
                        }
                        (KeyCode::PageUp, KeyModifiers::NONE) => {
                            list.selected = list.selected.saturating_sub(list.visible_height);
                        }
                        (KeyCode::Home, KeyModifiers::NONE) => {
                            list.selected = 0;
                        }
                        (KeyCode::End, KeyModifiers::NONE) => {
                            list.selected = list.tasks.len().saturating_sub(1);
                        }
                        (KeyCode::Char(' '), KeyModifiers::NONE) => {
                            //if let Err(e) = list.toggle() {
                            //    write!(&mut statusbar, "Error while changing status: {}", e)?;
                            //} else {
                            //    write!(&mut statusbar, "Task status changed")?;
                            //}
                        }

                        (KeyCode::Char('i'), KeyModifiers::NONE) => {
                            if let Some(t) = list.get() {
                                //show_task_details(&mut old, &mut new, &t.itask);
                                // create dialog with details
                            }
                        }
                        (KeyCode::Char('f'), KeyModifiers::NONE)
                        | (KeyCode::Char('f'), KeyModifiers::CONTROL)
                        | (KeyCode::F(4), KeyModifiers::NONE) => {
                            if let Some(filter) = common::ui::inline_editor(
                            &mut old,
                            &mut new,
                            10, // row
                            "Filter to apply - case insensitive, leave empty to show all tasks, use esc to abort input",
                            "> ",
                            &list.filter,
                        )? {
                        list.filter = filter.to_ascii_lowercase();
                        list.apply_filter();
                        list.selected = 0;
                    }
                        }

                        // need Z+shift to recognized z+shift on at least one env
                        (KeyCode::Char('Z'), KeyModifiers::SHIFT)
                        | (KeyCode::Char('Z'), KeyModifiers::NONE)
                        | (KeyCode::Char('z'), KeyModifiers::SHIFT) => {
                            paused = !paused;
                            needs_redraw = true;
                        }

                        (KeyCode::Char('p'), KeyModifiers::NONE) => {
                            fullpath = !fullpath;
                            list.reload(fullpath)?;
                            needs_redraw = true;
                        }

                        (KeyCode::Char('+'), KeyModifiers::NONE)
                        | (KeyCode::Char('-'), KeyModifiers::NONE) => {
                            if let Some(t) = list.get() {
                                match list.collapsed.binary_search(&t.pid) {
                                    Ok(index) => {
                                        list.collapsed.remove(index);
                                    }
                                    Err(index) => {
                                        list.collapsed.insert(index, t.pid);
                                    }
                                }
                                list.apply_filter();
                            }
                        }
                        _other => {
                            needs_redraw = false;
                        }
                    }
                }

                Event::Resize(width, height) => {
                    if new.width != width.into() || new.height != height.into() {
                        common::ui::resize_buffers(
                            &mut old,
                            &mut new,
                            width.into(),
                            height.into(),
                        )?;
                        needs_redraw = true;
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        list.selected = (list.selected + 1).min(list.tasks.len().saturating_sub(1));
                        needs_redraw = true;
                    }
                    MouseEventKind::ScrollUp => {
                        list.selected = list.selected.saturating_sub(1);
                        needs_redraw = true;
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        let mouse_row = mouse.row as usize;
                        // exclude header, status and empty rows
                        let toprows = headerbar.lines().count() + 1;
                        let bottomrows = 2;

                        if mouse_row >= toprows
                            && mouse_row - bottomrows < list.visible_height
                            && mouse_row - toprows + list.top < list.tasks.len()
                        {
                            list.selected = mouse_row - toprows + list.top;
                            needs_redraw = true;
                        }
                    }
                    _ => {}
                },

                _other => {}
            }
        }
    }
    Ok(())
}

pub(crate) fn add_virtual_root_prefixes(processes: &mut [ProcessInfo]) {
    use std::collections::HashSet;

    let known_pids: HashSet<u32> = processes.iter().map(|p| p.pid).collect();

    let root_indices: Vec<usize> = processes
        .iter()
        .enumerate()
        .filter(|(_, p)| p.ppid == 0 || !known_pids.contains(&p.ppid))
        .map(|(i, _)| i)
        .collect();

    let last_root = root_indices.last().copied();

    let mut only_space = false;
    for (i, process) in processes.iter_mut().enumerate() {
        let prefix = if only_space {
            "   "
        } else if root_indices.contains(&i) {
            if Some(i) == last_root {
                only_space = true;
                "└─ "
            } else {
                "├─ "
            }
        } else {
            "│  "
        };

        process.prefix.insert_str(0, prefix);
    }
}

impl List {
    fn new() -> Self {
        Self {
            alltasks: Vec::new(),
            tasks: Vec::new(),
            top: 0,
            selected: 0,
            visible_height: 0,
            filter: String::new(),
            collapsed: Vec::new(),
        }
    }

    fn reload(&mut self, fullpath: bool) -> Result<usize> {
        let wintasks = common::process::get_processes(fullpath).unwrap();
        let refs: Vec<&ProcessInfo> = wintasks.iter().collect();
        self.alltasks = common::process::process_tree_lines(&refs);

        add_virtual_root_prefixes(&mut self.alltasks);
        self.alltasks.insert(
            0,
            ProcessInfo {
                pid: 0,
                ppid: 0,
                name: "<Windows>".to_owned(),
                prefix: "".to_owned(),
                subsystem: Rc::from(""),
                cpu_usage_percent: 0.0,
                memory_usage_percent: 0.0,
                kernel_time: 0,
                user_time: 0,
                start_time_unix: None,
            },
        );

        let distros = common::wslprocess::get_distributions().unwrap();
        for d in distros {
            let dtasks = common::wslprocess::get_processes(&d).unwrap();
            let refs: Vec<&ProcessInfo> = dtasks.iter().collect();
            let mut tasks = common::process::process_tree_lines(&refs);
            add_virtual_root_prefixes(&mut tasks);
            tasks.insert(
                0,
                ProcessInfo {
                    pid: 0,
                    ppid: 0,
                    name: d,
                    prefix: "".to_owned(),
                    subsystem: Rc::from(""),
                    cpu_usage_percent: 0.0,
                    memory_usage_percent: 0.0,
                    kernel_time: 0,
                    user_time: 0,
                    start_time_unix: None,
                },
            );

            self.alltasks.append(&mut tasks);
        }

        self.apply_filter();
        self.selected = self.selected.min(self.tasks.len().saturating_sub(1));
        Ok(self.tasks.len())
    }
    fn apply_filter(&mut self) {
        let by_pid: std::collections::HashMap<u32, &ProcessInfo> =
            self.alltasks.iter().map(|task| (task.pid, task)).collect();

        self.tasks = self
            .alltasks
            .iter()
            .enumerate()
            .filter(|(_, task)| {
                task.name.to_ascii_lowercase().contains(&self.filter)
                    && !is_hidden_by_collapsed_ancestor(task, &by_pid, &self.collapsed)
            })
            .map(|(i, _)| i)
            .collect();
    }

    fn get(&self) -> Option<&ProcessInfo> {
        let idx = self.tasks.get(self.selected)?; // Option<&usize>
        self.alltasks.get(*idx) // Option<&Task>
    }
}

fn create_headerbar(buf: &common::ui::ScreenBuffer, list: &List, headerbar: &mut String) {
    let pid_width = list
        .alltasks
        .iter()
        .map(|r| r.pid.to_string().len())
        .max()
        .unwrap_or(3);

    let prefix_w = 2;
    let status_w = 10;
    let rem = buf.width - prefix_w - status_w - 2;
    let max_name = rem / 2;
    let max_path = rem - max_name;

    headerbar.clear();
    write!(
        headerbar,
        "Task Manager TUI\n{marker} {pid:>pid_width$} {cpu:>5} {mem:>5} {time:>8} {name:<name_width$} ",
        marker = ' ',
        cpu = "CPU%",
        mem = "MEM%",
        pid = "PID",
        time = "TIME+",
        name = common::ui::trim("Command", max_name),
        name_width = max_name,
    )
    .unwrap();
}

fn is_hidden_by_collapsed_ancestor(
    process: &ProcessInfo,
    by_pid: &std::collections::HashMap<u32, &ProcessInfo>,
    collapsed: &[u32],
) -> bool {
    let mut parent_pid = process.ppid;
    let mut visited = HashSet::new();

    while parent_pid != 0 && visited.insert(parent_pid) {
        if collapsed.contains(&parent_pid) {
            return true;
        }

        let Some(parent) = by_pid.get(&parent_pid) else {
            break;
        };

        parent_pid = parent.ppid;
    }

    false
}

fn draw_ui(buf: &mut common::ui::ScreenBuffer, list: &List, statusbar: &str, headerbar: &str) {
    if buf.width < 26 {
        let mut line = 0;
        buf.set_line(line, "Console too small");
        line += 1;

        while line < buf.height {
            buf.set_line(line, "");
            line += 1;
        }
        return;
    }
    // 1. Write header lines
    for (i, line) in headerbar.lines().enumerate() {
        buf.set_line(i, line);
    }

    let visible_height = buf.height.saturating_sub(headerbar.lines().count() + 2);
    if visible_height < 1 {
        return;
    }

    let mut row_y = headerbar.lines().count();

    let pid_width = list
        .alltasks
        .iter()
        .map(|r| r.pid.to_string().len())
        .max()
        .unwrap_or(3);

    let prefix_w = 2;
    let status_w = 10;
    let rem = buf.width - prefix_w - status_w - 2;
    let max_name = rem / 2;
    let max_path = rem - max_name;

    let sep = "-".repeat(buf.width);
    buf.set_line(row_y, &sep);
    row_y += 1;

    let visible_rows = list
        .tasks
        .iter()
        .skip(list.top)
        .take(visible_height)
        .map(|&i| &list.alltasks[i]);

    let mut index = list.top;

    for r in visible_rows {
        let has_children = list.alltasks.iter().any(|child| child.ppid == r.pid);
        let is_collapsed = list.collapsed.contains(&r.pid);

        let prefix = {
            if has_children && is_collapsed {
                r.prefix
                    .strip_suffix("─ ")
                    .map(|branch| format!("{branch}+ "))
                    .unwrap_or_else(|| r.prefix.clone())
            } else {
                r.prefix.clone()
            }
        };

        let centiseconds = r.kernel_time.saturating_add(r.user_time) / 100_000;
        let minutes = centiseconds / 6_000;
        let seconds = (centiseconds / 100) % 60;
        let hundredths = centiseconds % 100;

        let line = format!(
            "{marker} {pid:>pid_width$} {cpu:5.1} {mem:5.1} {time:>8} {prefix}{name:<name_width$}",
            marker = if index == list.selected { '>' } else { ' ' },
            pid = r.pid,
            cpu = r.cpu_usage_percent,
            mem = r.memory_usage_percent,
            time = format!("{minutes}:{seconds:02}.{hundredths:02}"),
            prefix = prefix,
            name = common::ui::trim(&r.name, max_name),
            name_width = max_name,
        );

        buf.set_line(row_y, &line);
        row_y += 1;
        index += 1;
    }

    // 3. Clear remaining rows
    while row_y < buf.height - 2 {
        buf.set_line(row_y, "");
        row_y += 1;
    }

    // 4. Separator line
    buf.set_line(buf.height - 2, &sep);

    // 5. Status line
    buf.set_line(buf.height - 1, &statusbar);
}
