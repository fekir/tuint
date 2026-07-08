use std::fmt::Write;

mod common;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};

use windows::{
    core::{Interface, BSTR},
    Win32::{
        NetworkManagement::WindowsFirewall::{
            INetFwPolicy2, INetFwRule, INetFwRules, NetFwPolicy2, NET_FW_ACTION_ALLOW,
            NET_FW_ACTION_BLOCK,
        },
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch, CLSCTX_INPROC_SERVER,
                COINIT_APARTMENTTHREADED,
            },
            Ole::IEnumVARIANT,
            Variant::VARIANT,
        },
    },
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

static HELP_TEXT: &str = //
    "Windows Fireall TUI

A TUI program for listing, enabling, disabling, and removing Windows Firewall rules

Command-line parameters

 --help                   - show this dialog


TUI options

Arrow Keys                - move up and down
Home/End                  - go to start/end of list
Q / F10 / Alt-0 / Ctrl+C  - close program
H / ? / F1 / Alt-1        - show this dialog
R / F5 / Ctrl+R / Alt-5   - refresh and redraw TUI
Canc/ F8 / Alt-8          - delete Windows Firewall rule
F / Ctrl+F                - filter tasks
Space                     - enable/disable Windows Firewall rule";

struct FirewallRule {
    name: String,
    action: String,
    program: String,
    protocoll: String,
    port: String,
    enabled: bool,
    rule: INetFwRule,
}

struct List {
    allrules: Vec<FirewallRule>,
    tasks: Vec<usize>,
    selected: usize,
    top: usize,
    visible_height: usize,
    filter: String,
    rules: INetFwRules,
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
        .map(|&i| &list.allrules[i]);

    let mut index = list.top;

    for r in visible_rows {
        let line = format!(
            "{marker} {name:<name_width$} {enabled} {action} {port:<20} {program}",
            marker = if index == list.selected { '>' } else { ' ' },
            name = common::ui::trim(&r.name, max_name),
            name_width = max_name,
            enabled = if r.enabled { "Enabled " } else { "Disabled" },
            action = r.action,
            port = common::ui::trim(&r.port, 20),
            program = common::ui::trim(&r.program, max_name),
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
    fn new(rules: INetFwRules) -> Self {
        Self {
            allrules: Vec::new(),
            tasks: Vec::new(),
            top: 0,
            selected: 0,
            visible_height: 0,
            filter: String::new(),
            rules: rules,
        }
    }

    fn reload(&mut self) -> Result<usize> {
        let res = list_all_rules(&self.rules, &mut self.allrules);
        self.apply_filter();
        self.selected = self.selected.min(self.tasks.len().saturating_sub(1));
        res?;
        Ok(0)
    }

    fn toggle(&mut self) -> Result<()> {
        if let Some(rule) = self.get() {
            let param = if (rule.enabled) {
                windows::Win32::Foundation::VARIANT_FALSE
            } else {
                windows::Win32::Foundation::VARIANT_TRUE
            };
            unsafe {
                rule.rule.SetEnabled(param)?;
            }
            self.reload()?;
        }
        Ok(())
    }

    fn delete(&mut self) -> Result<()> {
        if let Some(rule) = self.get() {
            unsafe {
                self.rules.Remove(&BSTR::from(&rule.name))?;
            }
            self.reload()?;
        }
        Ok(())
    }

    fn apply_filter(&mut self) {
        self.tasks = self
            .allrules
            .iter()
            .enumerate()
            .filter(|(_, task)| task.name.to_ascii_lowercase().contains(&self.filter))
            .map(|(i, _)| i)
            .collect();
    }

    fn get(&self) -> Option<&FirewallRule> {
        let idx = self.tasks.get(self.selected)?; // Option<&usize>
        self.allrules.get(*idx) // Option<&Task>
    }
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

    let rules = unsafe {
        let fwpolicy: INetFwPolicy2 = CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER)?;
        fwpolicy.Rules()?
    };

    //let mut rules = Vec::new();
    //list_all_rules(&fwpolicy, &mut rules)?;

    enable_raw_mode()?;

    crossterm::execute!(
        std::io::stdout(),
        crossterm::cursor::Hide,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    let res = run_tui(rules);

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

fn run_tui(rules: INetFwRules) -> Result<()> {
    let mut list = List::new(rules);
    list.reload()?;
    let mut needs_redraw = true;

    let (mut last_width, mut last_height) =
        crossterm::terminal::size().map(|(w, h)| (w as usize, h as usize))?;

    let mut old = common::ui::ScreenBuffer::new();
    let mut new = common::ui::ScreenBuffer::new();
    common::ui::resize_buffers(&mut old, &mut new, last_width, last_height)?;
    let mut statusbar = String::new();
    let headerbar = "Windows Firewall TUI";

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
                    | (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                        // first move goto to function
                        let width = new.width;
                        let height = new.height;
                        common::ui::resize_buffers(&mut old, &mut new, width, height)?;
                        match list.reload() {
                            Ok(v) => {
                                write!(&mut statusbar, "Loaded {} tasks", v)?;
                            }
                            Err(e) => {
                                write!(&mut statusbar, "Error loading Firewall rules: {}", e)?;
                            }
                        }
                    }

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
                            write!(&mut statusbar, "Firewall rule changed")?;
                        }
                    }

                    (KeyCode::F(8), KeyModifiers::NONE)
                    | (crossterm::event::KeyCode::Delete, KeyModifiers::NONE)
                    | (KeyCode::Char('8'), KeyModifiers::ALT) => {
                        if let Ok(Some(0)) = common::ui::choose_dialog(
                            &mut old,
                            &mut new,
                            "Do you want to delete",
                            &["Yes", "No"],
                        ) {
                            if let Err(e) = list.delete() {
                                write!(&mut statusbar, "Error while deleting firewall rule {}", e)?;
                            } else {
                                write!(&mut statusbar, "Firewall rule deleted")?;
                            }
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

fn list_all_rules(rules: &INetFwRules, allrules: &mut Vec<FirewallRule>) -> Result<()> {
    allrules.clear();

    let enum_variant = unsafe {
        //let count = rules.Count()?;
        let enum_variant = rules._NewEnum()?;
        let enum_variant: IEnumVARIANT = Interface::cast(&enum_variant)?;
        enum_variant
    };

    loop {
        let mut next = [VARIANT::default()]; // NOTE; can maybe use buffer > 1

        {
            let mut fetched: u32 = 0;
            let hr = unsafe { enum_variant.Next(&mut next, &mut fetched) };
            if hr.is_err() || fetched == 0 {
                break;
            }
        }

        let next = &next[0];
        let dispatch: &IDispatch =
            unsafe { next.Anonymous.Anonymous.Anonymous.pdispVal.as_ref() }.unwrap();

        let rule: INetFwRule = windows::core::Interface::cast(dispatch)?;

        let rule = unsafe {
            FirewallRule {
                name: rule.Name()?.to_string(),
                action: match rule.Action()? {
                    NET_FW_ACTION_ALLOW => "Allow".into(),
                    NET_FW_ACTION_BLOCK => "Block".into(),
                    _ => "???".into(),
                },
                program: rule.ApplicationName()?.to_string(),
                protocoll: rule.Protocol()?.to_string(),
                port: rule.LocalPorts()?.to_string(),
                enabled: rule.Enabled()?.as_bool(),
                rule: rule,
            }
        };
        allrules.push(rule);
    }
    allrules.sort_by_cached_key(|r| r.name.to_ascii_lowercase());

    Ok(())
}
