use std::fmt::Write;

mod common;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};

use windows::{
    core::Owned,
    Win32::System::Services::{
        SC_HANDLE, SERVICE_AUTO_START, SERVICE_BOOT_START, SERVICE_DEMAND_START, SERVICE_DISABLED,
        SERVICE_START_TYPE, SERVICE_SYSTEM_START,
    },
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

static HELP_TEXT: &str = //
    "Services TUI

A TUI program for listing, starting and stopping windows services

Command-line parameters

 --help                   - show this dialog


TUI options

Arrow Keys                - move up and down
Home/End                  - go to start/end of list
Alt-0 / Ctrl+C / F10 / Q  - close program
Alt-1 / F1 / ? / H        - show this dialog
Alt-5 / Ctrl+R / F5 / R   - refresh and redraw TUI
Ctrl+F / F                - filter services
Space                     - start/stop service";

fn clone_pwstring(ptr: windows::core::PWSTR) -> Vec<u16> {
    if ptr.0.is_null() {
        return vec![0];
    }
    let mut len = 0;
    unsafe {
        while *ptr.0.add(len) != 0 {
            len += 1;
        }

        std::slice::from_raw_parts(ptr.0, len + 1).to_vec()
    }
}

struct Service {
    name: String,
    internal_name: Vec<u16>, // PCWSTR
    startup_type: SERVICE_START_TYPE,
    is_running: bool,
}

struct List<'a> {
    allservices: Vec<Service>,
    services: Vec<usize>,
    selected: usize,
    top: usize,
    visible_height: usize,
    filter: String,
    scm: &'a SC_HANDLE,
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
        .services
        .iter()
        .skip(list.top)
        .take(visible_height)
        .map(|&i| &list.allservices[i]);

    let mut index = list.top;

    for r in visible_rows {
        let line = format!(
            "{marker} {name:<name_width$} {state:<10} {startup}",
            marker = if index == list.selected { '>' } else { ' ' },
            name = common::ui::trim(&r.name, max_name),
            name_width = max_name,
            state = if r.is_running { "Running" } else { "" },
            startup = match r.startup_type {
                SERVICE_BOOT_START => "Boot",
                SERVICE_SYSTEM_START => "System",
                SERVICE_AUTO_START => "Automatic",
                SERVICE_DEMAND_START => "Manual",
                SERVICE_DISABLED => "Disabled",
                _ => "Unknown",
            },
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

impl<'a> List<'a> {
    fn new(scm: &'a SC_HANDLE) -> Self {
        Self {
            allservices: Vec::new(),
            services: Vec::new(),
            top: 0,
            selected: 0,
            visible_height: 0,
            filter: String::new(),
            scm,
        }
    }

    fn reload(&mut self) -> Result<usize> {
        let res = list_all_services(self.scm, &mut self.allservices);
        self.apply_filter();
        self.selected = self.selected.min(self.services.len().saturating_sub(1));
        res?;
        Ok(self.services.len())
    }

    fn toggle(&mut self) -> Result<()> {
        use windows::Win32::System::Services::*;

        if let Some(task) = self.get() {
            let svc = unsafe {
                Owned::new(
                    OpenServiceW(
                        *self.scm,
                        windows::core::PCWSTR(task.internal_name.as_ptr()),
                        SERVICE_ALL_ACCESS,
                    )
                    .unwrap(),
                )
            };
            if task.is_running {
                let mut status: SERVICE_STATUS = Default::default();
                unsafe { ControlService(*svc, SERVICE_CONTROL_STOP, &mut status)? };
            } else {
                unsafe { StartServiceW(*svc, None)? }
            }

            let mut attempts = 0;
            let max_attempts = 2 * 500 * 5; // 5 seconds

            loop {
                let mut buffer = vec![0u8; std::mem::size_of::<SERVICE_STATUS_PROCESS>()];
                let mut bytes_needed = 0;

                let status = unsafe {
                    QueryServiceStatusEx(
                        *svc,
                        SC_STATUS_PROCESS_INFO,
                        Some(&mut buffer),
                        &mut bytes_needed,
                    )?;
                    &*(buffer.as_ptr() as *const SERVICE_STATUS_PROCESS)
                };

                if (task.is_running && status.dwCurrentState == SERVICE_STOPPED)
                    || (!task.is_running && status.dwCurrentState == SERVICE_RUNNING)
                {
                    break;
                }

                attempts += 1;
                if attempts >= max_attempts {
                    // timeout reached
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            self.reload()?;
        }

        Ok(())
    }

    fn apply_filter(&mut self) {
        self.services = self
            .allservices
            .iter()
            .enumerate()
            .filter(|(_, task)| task.name.to_ascii_lowercase().contains(&self.filter))
            .map(|(i, _)| i)
            .collect();
    }

    fn get(&self) -> Option<&Service> {
        let idx = self.services.get(self.selected)?; // Option<&usize>
        self.allservices.get(*idx) // Option<&Task>
    }
}

fn main() {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--help" | "-h" | "?" | "/?" => {
                println!("{}", HELP_TEXT);
                return;
            }
            other => {
                eprintln!("Unrecognized command-line parameter: {}", other);
                std::process::exit(1);
            }
        }
    }

    run_tui().unwrap();

    crossterm::execute!(std::io::stdout(), Clear(ClearType::All)).ok();
}

fn run_tui() -> Result<()> {
    enable_raw_mode()?;

    crossterm::execute!(
        std::io::stdout(),
        Clear(ClearType::All),
        crossterm::cursor::Hide,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    let scm = unsafe {
        Owned::new(windows::Win32::System::Services::OpenSCManagerW(
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR::null(),
            windows::Win32::System::Services::SC_MANAGER_ALL_ACCESS, //windows::Win32::System::Services::SC_MANAGER_ENUMERATE_SERVICE | windows::Win32::System::Services::SC_MANAGER_CONNECT,
        )?)
    };

    let mut list = List::new(&*scm);
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
                            write!(&mut statusbar, "Loaded {} services", v).unwrap();
                        }
                        Err(e) => {
                            write!(&mut statusbar, "Error loading task: {}", e).unwrap();
                        }
                    },

                    (KeyCode::Down, KeyModifiers::NONE) => {
                        list.selected =
                            (list.selected + 1).min(list.services.len().saturating_sub(1));
                    }
                    (KeyCode::Up, KeyModifiers::NONE) => {
                        list.selected = list.selected.saturating_sub(1);
                    }

                    (KeyCode::PageDown, KeyModifiers::NONE) => {
                        list.selected = (list.selected + list.visible_height)
                            .min(list.services.len().saturating_sub(1));
                    }
                    (KeyCode::PageUp, KeyModifiers::NONE) => {
                        list.selected = list.selected.saturating_sub(list.visible_height);
                    }
                    (KeyCode::Home, KeyModifiers::NONE) => {
                        list.selected = 0;
                    }
                    (KeyCode::End, KeyModifiers::NONE) => {
                        list.selected = list.services.len().saturating_sub(1);
                    }
                    (KeyCode::Char(' '), KeyModifiers::NONE) => {
                        // write status bar before, since startin/stopping service can take some time...
                        write!(&mut statusbar, "Updating status...").unwrap();
                        draw_ui(&mut new, &list, &statusbar, &headerbar);
                        common::ui::print_diff(&mut old, &mut new)?;
                        statusbar.clear();
                        if let Err(e) = list.toggle() {
                            write!(&mut statusbar, "Error while changing status: {}", e).unwrap();
                        } else {
                            write!(&mut statusbar, "Task status changed").unwrap();
                        }
                    }
                    (KeyCode::Char('f'), KeyModifiers::NONE)
                    | (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                        if let Some(filter) = common::ui::inline_editor(
                            &mut old,
                            &mut new,
                            10, // row
                            "Filter to apply - case insensitive, leave empty to show all services, use esc to abort input",
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
                        && mouse_row - toprows + list.top < list.services.len()
                    {
                        list.selected = mouse_row - toprows + list.top;
                        needs_redraw = true;
                    }
                }
                _ => {}
            },

            _other => {
                // for example keyup
                //counter3 += 1;
                //list.status = format!("event2 {:?}", other);
            }
        }
    }

    crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen
    )?;
    disable_raw_mode()?;

    Ok(())
}

fn list_all_services(
    scm: &windows::Win32::System::Services::SC_HANDLE,
    services: &mut Vec<Service>,
) -> Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Services::*;

    services.clear();
    {
        // Buffer for enumeration
        let mut bytes_needed = 0u32;
        let mut services_returned = 0u32;
        let mut resume_handle = 0u32;

        let status = unsafe {
            EnumServicesStatusExW(
                *scm,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                None,
                &mut bytes_needed,
                &mut services_returned,
                Some(&mut resume_handle),
                PCWSTR::null(),
            )
        };
        if let Err(err) = status {
            if err.code() == windows::Win32::Foundation::ERROR_MORE_DATA.to_hresult() {
                //println!("More data is available");
            } else {
                return Ok(());
            }
        }
        let mut buffer = vec![0u8; bytes_needed as usize];
        unsafe {
            EnumServicesStatusExW(
                *scm,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                Some(&mut buffer),
                &mut bytes_needed,
                &mut services_returned,
                Some(&mut resume_handle),
                PCWSTR::null(),
            )?
        };

        // Interpret buffer
        let service_status_proc = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW,
                services_returned as usize,
            )
        };
        for svc in service_status_proc {
            let name = svc.lpServiceName;
            let display = String::from_utf16_lossy(unsafe { svc.lpDisplayName.as_wide() }); // check if with as_wide can avoid alloc
            let status = svc.ServiceStatusProcess.dwCurrentState;
            //svc.ServiceStatusProcess.dwWin32ExitCode;

            // ---
            let svc =
                unsafe { Owned::new(OpenServiceW(*scm, svc.lpServiceName, SERVICE_QUERY_CONFIG)?) };

            let mut bytes_needed = 0u32;

            let _ = unsafe {
                QueryServiceConfigW(
                    *svc,
                    None, // lpServiceConfig
                    0,    // cbBufSize
                    &mut bytes_needed,
                )
            };
            let mut buffer = vec![0u8; bytes_needed as usize];
            let cfg = unsafe {
                QueryServiceConfigW(
                    *svc,
                    Some(buffer.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW),
                    bytes_needed,
                    &mut bytes_needed,
                )?;

                &*(buffer.as_ptr() as *const QUERY_SERVICE_CONFIGW)
            };

            //cfg.lpDependencies;
            let startype = cfg.dwStartType;
            // --

            services.push(Service {
                name: display,
                is_running: status == SERVICE_RUNNING,
                internal_name: clone_pwstring(name),
                startup_type: startype,
            });
        }
    }
    services.sort_by_cached_key(|t| t.name.to_ascii_lowercase());
    Ok(())
}
