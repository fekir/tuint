use std::fmt::Write;

mod common;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};

use windows::{
    core::BSTR,
    Win32::{
        Foundation::VARIANT_BOOL,
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_APARTMENTTHREADED,
            },
            TaskScheduler::{
                IRegisteredTask, ITaskFolder, ITaskService, TaskScheduler, TASK_ENUM_HIDDEN,
            },
            Variant::VARIANT,
        },
    },
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

static HELP_TEXT: &str = //
    "Task Manager TUI

A TUI program for listing, enabling, and disabling scheduled tasks

Command-line parameters

 --help                   - show this dialog


TUI options

Arrow Keys                - move up and down
Home/End                  - go to start/end of list
Alt-0 / Ctrl+C / F10 / Q  - close program
Alt-1 / F1 / ? / H        - show this dialog
Alt-5 / Ctrl+R / F5 / R   - refresh and redraw TUI
Ctrl+F / F                - filter tasks
I                         - show additional information about the selected task
Space                     - enable/disable scheduled task";

struct Task {
    name: String,
    path: String,
    enabled: bool,
    itask: IRegisteredTask,
}

struct List {
    alltasks: Vec<Task>,
    tasks: Vec<usize>,
    selected: usize,
    top: usize,
    visible_height: usize,
    filter: String,
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

    let sep = "-".repeat(buf.width);
    buf.set_line(row_y, &sep);
    row_y += 1;

    let prefix_w = 2;
    let status_w = 10;
    let rem = buf.width - prefix_w - status_w - 2;
    let max_name = rem / 2;
    let max_path = rem - max_name;

    let visible_rows = list
        .tasks
        .iter()
        .skip(list.top)
        .take(visible_height)
        .map(|&i| &list.alltasks[i]);

    let mut index = list.top;

    for r in visible_rows {
        let line = format!(
            "{marker} {name:<name_width$} {state:<10} {path:<max_path$}",
            marker = if index == list.selected { '>' } else { ' ' },
            name = common::ui::trim(&r.name, max_name),
            name_width = max_name,
            state = if r.enabled { "Enabled" } else { "Disabled" },
            path = common::ui::trim(&r.path, max_path).replace('\\', "/"),
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

impl List {
    fn new() -> Self {
        Self {
            alltasks: Vec::new(),
            tasks: Vec::new(),
            top: 0,
            selected: 0,
            visible_height: 0,
            filter: String::new(),
        }
    }

    fn reload(&mut self) -> Result<usize> {
        let res = list_all_tasks(&mut self.alltasks);
        self.apply_filter();
        self.selected = self.selected.min(self.tasks.len().saturating_sub(1));
        res?;
        Ok(self.tasks.len())
    }

    fn toggle(&mut self) -> Result<()> {
        if let Some(task) = self.get() {
            unsafe {
                task.itask.SetEnabled(VARIANT_BOOL::from(!task.enabled))?;
            }
            self.reload()?;
        }
        Ok(())
    }

    fn apply_filter(&mut self) {
        self.tasks = self
            .alltasks
            .iter()
            .enumerate()
            .filter(|(_, task)| task.name.to_ascii_lowercase().contains(&self.filter))
            .map(|(i, _)| i)
            .collect();
    }

    fn get(&self) -> Option<&Task> {
        let idx = self.tasks.get(self.selected)?; // Option<&usize>
        self.alltasks.get(*idx) // Option<&Task>
    }
}

use windows::Win32::Foundation::SYSTEMTIME;

use crate::common::ui::info_dialog;

fn vt_date_to_systemtime(date: f64) -> SYSTEMTIME {
    let mut st = SYSTEMTIME::default();
    let ok = unsafe { windows::Win32::System::Variant::VariantTimeToSystemTime(date, &mut st) };
    if ok != 0 {
        st
    } else {
        SYSTEMTIME::default()
    }
}

fn get_bstr<F>(f: F) -> windows::core::Result<String>
where
    F: FnOnce(*mut BSTR) -> windows::core::Result<()>,
{
    let mut b: BSTR = windows::core::BSTR::new();
    f(&mut b)?;
    Ok(b.to_string())
}

fn com_count<F>(f: F) -> windows::core::Result<i32>
where
    F: FnOnce(*mut i32) -> windows::core::Result<()>,
{
    let mut count = 0i32;
    f(&mut count)?;
    Ok(count)
}
fn com_bool<F>(f: F) -> windows::core::Result<bool>
where
    F: FnOnce(*mut VARIANT_BOOL) -> windows::core::Result<()>,
{
    let mut b = VARIANT_BOOL(0);
    f(&mut b)?;
    Ok(b.as_bool())
}

fn format_systemtime(st: &SYSTEMTIME) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond,
    )
}

fn win32_message(code: u32) -> String {
    use windows::Win32::System::Diagnostics::Debug::FormatMessageW;
    use windows::Win32::System::Diagnostics::Debug::FORMAT_MESSAGE_FROM_SYSTEM;

    let mut buf = [0u16; 512];
    let len = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM,
            None,
            code,
            0,
            windows::core::PWSTR(buf.as_mut_ptr()),
            buf.len() as u32,
            None,
        )
    };
    String::from_utf16_lossy(&buf[..len as usize])
}

fn show_task_details(
    old: &mut common::ui::ScreenBuffer,
    new: &mut common::ui::ScreenBuffer,
    task: &IRegisteredTask,
) {
    let mut message = String::new();
    unsafe {
        let _ = writeln!(
            message,
            "Name:            {}",
            task.Name()
                .map(|b| b.to_string())
                .unwrap_or_else(|_| "<unknown>".to_string()),
        );
        let _ = writeln!(
            message,
            "Location:        {}",
            task.Path()
                .map(|b| b.to_string())
                .unwrap_or_else(|_| "<unknown>".to_string()),
        );
        let _ = writeln!(
            message,
            "Status:          {}",
            task.Enabled()
                .map(|b| if b.as_bool() { "Enabled" } else { "Disabled" })
                .unwrap_or_else(|_| "<unknown>"),
        );

        match task.Definition() {
            Err(e) => {
                let _ = writeln!(message, "Unable to query task definition ({})", e);
            }
            Ok(def) => {
                match def.RegistrationInfo() {
                    Err(e) => {
                        let _ = writeln!(message, "Unable to query task registrationinfo ({})", e);
                    }
                    Ok(info) => {
                        let _ = writeln!(
                            message,
                            "Author:          {}",
                            get_bstr(|p| info.Author(p)).unwrap_or("N/A".to_string()),
                        );
                        let _ = writeln!(
                            message,
                            "Description:     {}",
                            get_bstr(|p| info.Description(p)).unwrap_or("N/A".to_string()),
                        );
                    }
                };

                match def.Triggers() {
                    Err(e) => {
                        let _ = writeln!(message, "Unable to query triggers:   {}", e);
                    }
                    Ok(triggers) => {
                        let count = com_count(|p| triggers.Count(p));
                        match count {
                            Err(e) => {
                                let _ = writeln!(message, "Trigger count:   {}", e);
                            }
                            Ok(count) => {
                                let _ = writeln!(message, "Trigger count:   {}", count);
                                for i in 1..=count {
                                    let trigger = match triggers.get_Item(i) {
                                        Ok(a) => a,
                                        Err(_) => {
                                            let _ =
                                                writeln!(message, "Unable to get item:   {}", i);
                                            continue;
                                        }
                                    };

                                    use windows::Win32::System::TaskScheduler::*;

                                    let mut ttype = TASK_TRIGGER_TYPE2(0); // allocate storage
                                    trigger.Type(&mut ttype).unwrap_unchecked();

                                    let _ = writeln!(
                                        message,
                                        " Trigger type:   {}",
                                        match ttype {
                                            TASK_TRIGGER_EVENT => "Event",
                                            TASK_TRIGGER_TIME => "Time",
                                            TASK_TRIGGER_DAILY => "Daily",
                                            TASK_TRIGGER_WEEKLY => "Weekly",
                                            TASK_TRIGGER_MONTHLY => "Monthly",
                                            TASK_TRIGGER_MONTHLYDOW => "Monthly Day-of-Week",
                                            TASK_TRIGGER_IDLE => "Idle",
                                            TASK_TRIGGER_REGISTRATION => "Registration",
                                            TASK_TRIGGER_BOOT => "Boot",
                                            TASK_TRIGGER_LOGON => "Logon",
                                            TASK_TRIGGER_SESSION_STATE_CHANGE =>
                                                "Session State Change",
                                            _ => "Unknown",
                                        }
                                    );

                                    let start_boundary = get_bstr(|p| trigger.StartBoundary(p));
                                    let end_boundary = get_bstr(|p| trigger.EndBoundary(p));

                                    let _ = writeln!(
                                        message,
                                        " StartBoundary:  {}",
                                        start_boundary.unwrap_or("N/A".to_string()),
                                    );
                                    let _ = writeln!(
                                        message,
                                        " EndBoundary:    {}",
                                        end_boundary.unwrap_or("N/A".to_string()),
                                    );
                                }
                            }
                        }
                    }
                }
                match def.Actions() {
                    Err(e) => {
                        let _ = writeln!(message, "Unable to query actions ({})", e);
                    }
                    Ok(actions) => {
                        let count = com_count(|p| actions.Count(p));
                        match count {
                            Err(e) => {
                                let _ = writeln!(message, "Actions count:   {}", e);
                            }
                            Ok(count) => {
                                let _ = writeln!(message, "Action count:    {}", count);
                                for i in 1..=count {
                                    let action = match actions.get_Item(i) {
                                        Ok(a) => a,
                                        Err(_) => {
                                            let _ =
                                                writeln!(message, "Unable to get item:   {}", i);
                                            continue;
                                        }
                                    };
                                    if let Ok(exec) = windows::core::Interface::cast::<
                                        windows::Win32::System::TaskScheduler::IExecAction,
                                    >(&action)
                                    {
                                        let path = get_bstr(|p| exec.Path(p));
                                        let args = get_bstr(|p| exec.Arguments(p));
                                        let workdir = get_bstr(|p| exec.WorkingDirectory(p));

                                        let _ = writeln!(
                                            message,
                                            " Exec Action:
   Program:      {}
   Arguments:    {}
   WorkingDir:   {}",
                                            path.unwrap_or_default(),
                                            args.unwrap_or_default(),
                                            workdir.unwrap_or_default()
                                        );
                                    } else {
                                        let _ = writeln!(message, "Non-exec action (unsupported)");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let _ = writeln!(
            message,
            "Next run:        {}",
            format_systemtime(&vt_date_to_systemtime(
                task.NextRunTime().unwrap_or_default()
            ))
        );
        match task.LastTaskResult() {
            Ok(v) => {
                let _ = writeln!(
                    message,
                    "Last result:     {} (0x{:08X})",
                    win32_message(v as u32).trim(),
                    v,
                );
            }
            Err(e) => {
                let _ = writeln!(message, "Last result:     {}", e);
            }
        }
    };
    let _ = info_dialog(old, new, &message);
}

fn main() -> Result<()> {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--help" | "-h" | "?" | "/?" => {
                println!("{}", HELP_TEXT);
                return Ok(());
            }
            other => {
                return Err(format!("Unrecognized command-line parameter: {}", other).into());
            }
        }
    }

    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr.is_err() {
        return Err("Unable to initialize COM interface".into());
    }

    enable_raw_mode()?;

    crossterm::execute!(
        std::io::stdout(),
        Clear(ClearType::All),
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
    unsafe {
        CoUninitialize();
    }
    return res;
}

fn run_tui() -> Result<()> {
    let mut list = List::new();
    list.reload()?;
    let mut needs_redraw = true;

    let mut old = common::ui::ScreenBuffer::new(0, 0);
    let mut new = common::ui::ScreenBuffer::new(0, 0);
    common::ui::resize_buffers(&mut old, &mut new, 0, 0)?;
    let (mut last_width, mut last_height) =
        crossterm::terminal::size().map(|(w, h)| (w as usize, h as usize))?;

    let mut statusbar = String::new();
    let headerbar = "Task Scheduler TUI";

    loop {
        if needs_redraw {
            if last_width != new.width || last_height != new.height {
                last_width = new.width;
                last_height = new.height;
            }

            list.visible_height = old.height.saturating_sub(3 + headerbar.lines().count()) as usize;
            list.top = list.top.min(list.selected);
            list.top = list.top.max(
                list.selected
                    .saturating_sub(list.visible_height.saturating_sub(1)),
            );
            draw_ui(&mut new, &list, &statusbar, &headerbar);
            common::ui::print_diff(&mut old, &mut new)?;
            needs_redraw = false;
            statusbar.clear();
        }

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
                    | (KeyCode::Char('r'), KeyModifiers::CONTROL) => match list.reload() {
                        Ok(v) => {
                            write!(&mut statusbar, "Loaded {} tasks", v)?;
                        }
                        Err(e) => {
                            write!(&mut statusbar, "Error loading task: {}", e)?;
                        }
                    },

                    (KeyCode::Down, KeyModifiers::NONE) => {
                        list.selected = (list.selected + 1).min(list.tasks.len().saturating_sub(1));
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
                        if let Err(e) = list.toggle() {
                            write!(&mut statusbar, "Error while changing status: {}", e)?;
                        } else {
                            write!(&mut statusbar, "Task status changed")?;
                        }
                    }

                    (KeyCode::Char('i'), KeyModifiers::NONE) => {
                        if let Some(t) = list.get() {
                            show_task_details(&mut old, &mut new, &t.itask);
                            // create dialog with details
                        }
                    }
                    (KeyCode::Char('f'), KeyModifiers::NONE)
                    | (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
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
                    _other => {
                        needs_redraw = false;
                    }
                }
            }

            Event::Resize(width, height) => {
                if new.width != width.into() || new.height != height.into() {
                    common::ui::resize_buffers(&mut old, &mut new, width.into(), height.into())?;
                    needs_redraw = true;
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
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

    Ok(())
}

fn list_all_tasks(tasks: &mut Vec<Task>) -> Result<()> {
    tasks.clear();
    unsafe {
        let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
        let empty = VARIANT::default();
        service.Connect(&empty, &empty, &empty, &empty)?;
        let root = service.GetFolder(&BSTR::from("\\"))?;
        enumerate_folder_tasks(&root, "\\", tasks)?;
    }
    tasks.sort_by_cached_key(|t| (t.path.to_ascii_lowercase(), t.name.to_ascii_lowercase()));
    Ok(())
}

unsafe fn enumerate_folder_tasks(
    folder: &ITaskFolder,
    path: &str,
    out: &mut Vec<Task>,
) -> Result<()> {
    let collection: windows::Win32::System::TaskScheduler::IRegisteredTaskCollection =
        folder.GetTasks(TASK_ENUM_HIDDEN.0 as i32)?;

    for i in 1..=collection.Count()? {
        let task: IRegisteredTask = collection.get_Item(&VARIANT::from(i as i32))?;

        out.push(Task {
            name: task.Name()?.to_string(),
            path: path.to_string(),
            enabled: task.Enabled()?.as_bool(),
            itask: task,
        });
    }

    let subfolders = folder.GetFolders(0)?;
    for i in 1..=subfolders.Count()? {
        let sub: ITaskFolder = subfolders.get_Item(&VARIANT::from(i as i32))?;
        let new_path = format!("{}{}\\", path, sub.Name()?);
        enumerate_folder_tasks(&sub, &new_path, out)?;
    }

    Ok(())
}
