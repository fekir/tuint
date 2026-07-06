use std::fmt::Write;

mod common;
use common::error::win32_error_to_boxed;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
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
Alt-4 / F4                - edit Registry Value
Alt-5 / Ctrl+R / F5 / R   - refresh and redraw TUI
Alt-7 / F7                - create Registry Key or Value
Alt-8 / F8                - delete Registry Key or Value
Ctrl+F / F                - filter tasks
Enter                     - open selected Registry Key";

#[derive(Copy, Clone, PartialEq, Eq)]
enum RegOrStr {
    RegType(REG_VALUE_TYPE),
    RegType2(Additionalreg),
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Additionalreg {
    Up,
    Key,
}

struct RegistryRow {
    name: String,
    value_display: String,
    value_kind: RegOrStr,
}

// FIXME: rename to list, should handle list of elements -> remove header
// should make it easier to avoid opies and split tasks

struct List {
    alltasks: Vec<RegistryRow>,
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
    let type_width = 20;
    // fixme: consider screen size
    let name_width = list
        .alltasks
        .iter()
        .map(|r| r.name.len())
        .max()
        .unwrap_or(0)
        + 5;
    //.min(buf.width - 3 - prefix_w - type_len - status_w - 2);
    // and split with value

    let visible_rows = list
        .tasks
        .iter()
        .skip(list.top)
        .take(visible_height)
        .map(|&i| &list.alltasks[i]);

    let mut index = list.top;

    for r in visible_rows {
        let line = format!(
            "{marker} {name:<name_width$} {value_kind:<type_width$} {value}",
            marker = if index == list.selected { '>' } else { ' ' },
            name = common::ui::trim(&r.name, name_width),
            name_width = name_width,
            value_kind = match r.value_kind {
                RegOrStr::RegType2(_) => "",
                RegOrStr::RegType(REG_DWORD) => "DWord",
                RegOrStr::RegType(REG_DWORD_BIG_ENDIAN) => "DWord (big endian)",
                RegOrStr::RegType(REG_QWORD) => "QWord",
                RegOrStr::RegType(REG_BINARY) => "Binary",
                RegOrStr::RegType(REG_SZ) => "String",
                RegOrStr::RegType(REG_EXPAND_SZ) => "String(String)",
                RegOrStr::RegType(REG_MULTI_SZ) => "Multiline String",
                RegOrStr::RegType(REG_NONE) => "NONE",
                RegOrStr::RegType(_) => "???",
            },
            type_width = type_width,
            value = r.value_display
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

    fn apply_filter(&mut self) {
        self.tasks = self
            .alltasks
            .iter()
            .enumerate()
            .filter(|(i, task)| *i == 0 || task.name.to_ascii_lowercase().contains(&self.filter))
            .map(|(i, _)| i)
            .collect();
    }

    fn loadkey(&mut self, key: HKEY, statusbar: &mut String, resetfilter: bool) {
        if let Err(e) = get_all_registry_rows(key, &mut self.alltasks) {
            write!(statusbar, "unable to reload key {}", e).unwrap();
        }
        // also in case of error, since self.alltasks might have changed, need to update tasks
        self.tasks = (0..self.alltasks.len()).collect();
        self.selected = 0;
        if resetfilter {
            self.filter.clear();
        }
        self.apply_filter();
    }

    fn get(&self) -> &RegistryRow {
        &self.alltasks[*&self.tasks[self.selected]]
    }
}

fn parse_path(mut path: String) -> (Option<Hive>, String) {
    // use abbreviations
    const FULL: &[(&str, &str)] = &[
        ("HKEY_LOCAL_MACHINE", "HKLM"),
        ("HKEY_CURRENT_USER", "HKCU"),
        ("HKEY_CLASSES_ROOT", "HKCR"),
        ("HKEY_USERS", "HKU"),
        ("HKEY_CURRENT_CONFIG", "HKCC"),
    ];

    for (full, short) in FULL {
        if path.starts_with(full) {
            path = path.replacen(full, short, 1);
        }
    }

    // use : after hive, and remove optional separator
    const HIVES: &[&str] = &["HKLM", "HKCU", "HKCR", "HKU", "HKCC"];
    for hive in HIVES {
        let prefix1 = format!("{}:\\", hive);
        let prefix2 = format!("{}\\", hive);
        //let prefix1 = format!("{}:/", hive);
        //let prefix4 = format!("{}/", hive);
        let newprefix = format!("{}:", hive);

        if path.starts_with(&prefix1) {
            path = newprefix.clone() + &path[prefix1.len()..];
        } else if path.starts_with(&prefix2) {
            path = newprefix.clone() + &path[prefix2.len()..];
        }
    }

    // extract hive
    let registry_hives = ["HKLM", "HKCU", "HKCR", "HKU", "HKCC"];
    let drive = path.split(':').next().unwrap_or("").to_string();
    let hive_index = registry_hives.iter().position(|h| *h == drive);

    let hive = match hive_index {
        Some(0) => Some(HKLM),
        Some(1) => Some(HKCU),
        Some(2) => Some(HKCR),
        Some(3) => Some(HKU),
        Some(4) => Some(HKCC),
        _ => None,
    };

    let no_qualifier = path.splitn(2, ':').nth(1).unwrap_or("").to_string();
    path = no_qualifier;

    let path = path.trim_end_matches('\\').to_string();

    return (hive, path);
}

fn main() -> Result<()> {
    let mut path = String::new();
    let mut hive = HKCU;
    let mut args = std::env::args().skip(1).peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" | "?" | "/?" => {
                println!("{}", HELP_TEXT);
                return Ok(());
            }

            "--path" => {
                // Look ahead for the value
                if let Some(value) = args.next() {
                    let (new_hive, new_path) = parse_path(value.clone());
                    if let Some(new_hive) = new_hive {
                        hive = new_hive;
                        path = new_path;
                    } else {
                        return Err(format!("Invalid registry hive in path: {}", value).into());
                    }
                } else {
                    return Err("Missing value for --path".into());
                }
            }
            other => {
                return Err(format!("Unrecognized command-line parameter: {}", other).into());
            }
        }
    }

    enable_raw_mode()?;

    crossterm::execute!(
        std::io::stdout(),
        Clear(ClearType::All),
        crossterm::cursor::Hide,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    let res = run_tui(hive, path);

    crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen,
        Clear(ClearType::All),
    )?;
    disable_raw_mode()?;

    return res;
}

fn run_tui(mut hive: Hive, mut path: String) -> Result<()> {
    // let key = open_registry_key(Hive::HKLM, None, false)?;
    //let key = open_registry_key(Hive::HKCU, Some("Control Panel"), false)?;
    let writable = true;
    let mut current_hkey = common::regedit::open_registry_key(hive.hkey, &path, writable)?;

    let mut needs_redraw = true;

    let mut old = common::ui::ScreenBuffer::new(0, 0);
    let mut new = common::ui::ScreenBuffer::new(0, 0);
    common::ui::resize_buffers(&mut old, &mut new, 0, 0)?;
    let (mut last_width, mut last_height) =
        crossterm::terminal::size().map(|(w, h)| (w as usize, h as usize))?;

    let mut list = List::new();
    let mut statusbar = String::new();
    let mut headerbar = String::new();
    list.loadkey(current_hkey, &mut statusbar, true);

    loop {
        if needs_redraw {
            if last_width != new.width || last_height != new.height {
                last_width = new.width;
                last_height = new.height;
            }
            headerbar.clear();
            write!(&mut headerbar, "Regedit TUI\n {}:\\{}", hive.str, &path)?;

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
                    | (KeyCode::Char('5'), KeyModifiers::ALT)
                    | (KeyCode::Char('r'), KeyModifiers::NONE)
                    | (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                        // first move goto to function
                        list.loadkey(current_hkey, &mut statusbar, false);
                        write!(&mut statusbar, "reloaded")?;
                    }

                    (KeyCode::Char('g'), KeyModifiers::NONE) => {
                        match common::ui::inline_editor(
                            &mut old,
                            &mut new,
                            10, // row
                            &format!("Insert new location to go"),
                            "> ",
                            &format!("{}:\\{}", hive.str, &path),
                        )? {
                            Some(new_location) => {
                                let (new_hive, mut new_path) = parse_path(new_location.clone());
                                if let Some(new_hive) = new_hive {
                                    goto(
                                        new_hive,
                                        &mut hive,
                                        &mut current_hkey,
                                        &mut new_path,
                                        &mut path,
                                        writable,
                                        &mut statusbar,
                                        &mut list,
                                    );
                                } else {
                                    write!(
                                        &mut statusbar,
                                        "Invalid registry hive in path: {}",
                                        new_location
                                    )?;
                                }
                            }
                            None => {
                                statusbar.push_str("Cancelled");
                                continue;
                            }
                        };
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
                    (KeyCode::Enter, KeyModifiers::NONE) => {
                        let item = list.get();
                        match item.value_kind {
                            RegOrStr::RegType2(Additionalreg::Up) => {
                                if !path.is_empty() {
                                    // Navigate to parent
                                    let mut parent = path
                                        .rsplit_once('\\')
                                        .map(|(p, _)| p.to_string())
                                        .unwrap_or_default();
                                    goto(
                                        hive,
                                        &mut hive,
                                        &mut current_hkey,
                                        &mut parent,
                                        &mut path,
                                        writable,
                                        &mut statusbar,
                                        &mut list,
                                    );
                                } else {
                                    let res = common::ui::choose_dialog(
                                        &mut old,
                                        &mut new,
                                        "Choose hive",
                                        &["HKLM", "HKCU", "HKCR", "HKU", "HKCC"],
                                    );
                                    let new_hive = match res {
                                        Ok(Some(0)) => HKLM,
                                        Ok(Some(1)) => HKCU,
                                        Ok(Some(2)) => HKCR,
                                        Ok(Some(3)) => HKU,
                                        Ok(Some(4)) => HKCC,
                                        _ => {
                                            write!(&mut statusbar, "Unexcpected status")?;
                                            continue;
                                        }
                                    };
                                    goto(
                                        new_hive,
                                        &mut hive,
                                        &mut current_hkey,
                                        &mut "".into(),
                                        &mut path,
                                        writable,
                                        &mut statusbar,
                                        &mut list,
                                    );
                                }
                            }
                            RegOrStr::RegType2(Additionalreg::Key) => {
                                // Build subkey path
                                let mut sub = if path.is_empty() {
                                    item.name.clone()
                                } else {
                                    format!("{}\\{}", path, item.name)
                                };
                                goto(
                                    hive,
                                    &mut hive,
                                    &mut current_hkey,
                                    &mut sub,
                                    &mut path,
                                    writable,
                                    &mut statusbar,
                                    &mut list,
                                );
                            }
                            _ => {}
                        }
                    }

                    (KeyCode::F(4), KeyModifiers::NONE)
                    | (KeyCode::Char('4'), KeyModifiers::ALT) => {
                        if !writable {
                            write!(&mut statusbar, "not writable, operation disabled")?;
                            continue;
                        }
                        let item = list.get();
                        match item.value_kind {
                            RegOrStr::RegType(reg_value_type) => {
                                let res = edit_registry_value(
                                    &mut old,
                                    &mut new,
                                    current_hkey,
                                    &item.value_display,
                                    &item.name,
                                    reg_value_type,
                                    &mut statusbar,
                                )?;
                                if res {
                                    let selected = list.selected;
                                    let filter = list.filter.clone();
                                    list.loadkey(current_hkey, &mut statusbar, false);
                                    list.selected = selected;
                                    list.filter = filter;
                                    list.apply_filter();
                                }
                            }
                            _ => {}
                        }
                    }

                    (KeyCode::F(6), KeyModifiers::NONE)
                    | (KeyCode::Char('6'), KeyModifiers::ALT) => {
                        if !writable {
                            write!(&mut statusbar, "not writable, operation disabled")?;
                            continue;
                        }
                        let item = list.get();

                        // get text and function pointer to avoid repeating match
                        type RenameFn = fn(HKEY, &str, &str) -> Result<()>;
                        let (key_or_value, fun) = match item.value_kind {
                            RegOrStr::RegType(_) => {
                                ("value", common::regedit::rename_value as RenameFn)
                            }
                            RegOrStr::RegType2(Additionalreg::Key) => {
                                ("value", common::regedit::rename_key as RenameFn)
                            }
                            RegOrStr::RegType2(Additionalreg::Up) => {
                                write!(&mut statusbar, "Unable to rename current element")?;
                                continue;
                            }
                        };

                        let new_name = match common::ui::inline_editor(
                            &mut old,
                            &mut new,
                            10, // row
                            &format!("Insert new name for registry {key_or_value}"),
                            "> ",
                            &item.name,
                        )? {
                            Some(v) => v,
                            None => {
                                statusbar.push_str("Cancelled");
                                continue;
                            }
                        };
                        if let Err(err) = fun(current_hkey, &item.name, &new_name) {
                            write!(&mut statusbar, "Failed to rename {key_or_value}: {err}")?;
                        } else {
                            let sel = list.selected;
                            list.loadkey(current_hkey, &mut statusbar, true);
                            list.selected = sel;
                        }
                    }

                    (KeyCode::F(7), KeyModifiers::NONE)
                    | (KeyCode::Char('7'), KeyModifiers::ALT) => {
                        if !writable {
                            write!(&mut statusbar, "not writable, operation disabled")?;
                            continue;
                        }

                        let buttons = [
                            "Key",
                            "String Value",
                            "Dword Value",
                            "Qword Value",
                            "Expandable String Value",
                            // -begin- editing of those not supported yet, but keep for testing
                            "Binary Value",
                            "Multiline String Value",
                            "None Value",
                            // -end-
                            "Big-Endian Dword Value",
                            "Volatile Key",
                            "Cancel",
                        ];
                        let res = common::ui::choose_dialog(
                            &mut old,
                            &mut new,
                            "Choose what to create",
                            &buttons,
                        );
                        match res {
                            Ok(Some(i)) => match buttons[i] {
                                "Cancel" => {
                                    continue;
                                }
                                "Key" | "Volatile Key" => {
                                    if let Some(new_path) = common::ui::inline_editor(
                                        &mut old,
                                        &mut new,
                                        10, // row
                                        "Insert name of new Key. Esc to cancel",
                                        "> ",
                                        &list.filter,
                                    )? {
                                        if let Err(err) = common::regedit::create_key(
                                            current_hkey,
                                            &new_path,
                                            buttons[i] == "Key(Volatile)",
                                        ) {
                                            write!(
                                                &mut statusbar,
                                                "Unable to create key: {}",
                                                err
                                            )?;
                                        }
                                    }
                                }
                                _ => {
                                    if let Some(new_value) = common::ui::inline_editor(
                                        &mut old,
                                        &mut new,
                                        10, // row
                                        "Insert name of new Value. Esc to cancel",
                                        "> ",
                                        &list.filter,
                                    )? {
                                        let rtype = match buttons[i] {
                                            "String Value" => REG_SZ,
                                            "Dword Value" => REG_DWORD,
                                            "Big-Endian Dword Value" => REG_DWORD_BIG_ENDIAN,
                                            "Qword Value" => REG_QWORD,
                                            "Binary Value" => REG_BINARY,
                                            "None Value" => REG_NONE,
                                            "Multiline String Value" => REG_MULTI_SZ,
                                            "Expandable String Value" => REG_EXPAND_SZ,
                                            _ => continue,
                                        };

                                        let default = match rtype {
                                            REG_DWORD | REG_QWORD | REG_DWORD_BIG_ENDIAN => "0",
                                            _ => "",
                                        };

                                        edit_registry_value(
                                            &mut old,
                                            &mut new,
                                            current_hkey,
                                            default,
                                            &new_value,
                                            rtype,
                                            &mut statusbar,
                                        )?;
                                    }
                                }
                            },
                            Ok(None) => {}
                            Err(e) => {
                                write!(&mut statusbar, "Unexpected error: {}", e)?;
                            }
                        };
                        list.loadkey(current_hkey, &mut statusbar, true);
                    }

                    (KeyCode::F(8), KeyModifiers::NONE)
                    | (KeyCode::Char('8'), KeyModifiers::ALT) => {
                        if !writable {
                            write!(&mut statusbar, "not writable, operation disabled")?;
                            continue;
                        }
                        let item = list.get();
                        match item.value_kind {
                            RegOrStr::RegType2(Additionalreg::Up) => {
                                write!(&mut statusbar, "not writable, operation disabled")?;
                                continue;
                            }
                            _ => {
                                let name = item.name.clone();

                                if let Err(err) =
                                    common::regedit::delete_key_or_value(current_hkey, &item.name)
                                {
                                    write!(&mut statusbar, "Failed to delete {} - {}", name, err)?;
                                }
                            }
                        }

                        let selected = list.selected;
                        list.loadkey(current_hkey, &mut statusbar, false);
                        list.selected = selected.min(list.tasks.len().saturating_sub(1));
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
                        list.selected=0;
                    }
                    }
                    _other => {
                        needs_redraw = false;
                    }
                }
            }

            Event::Resize(width, height) => {
                if last_width != width.into() || last_height != height.into() {
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

#[derive(Copy, Clone)]
pub struct Hive {
    pub hkey: windows::Win32::System::Registry::HKEY,
    pub str: &'static str,
}

pub const HKLM: Hive = Hive {
    hkey: windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
    str: "HKLM",
};
pub const HKCU: Hive = Hive {
    hkey: windows::Win32::System::Registry::HKEY_CURRENT_USER,
    str: "HKCU",
};
pub const HKCR: Hive = Hive {
    hkey: windows::Win32::System::Registry::HKEY_CLASSES_ROOT,
    str: "HKCR",
};
pub const HKU: Hive = Hive {
    hkey: windows::Win32::System::Registry::HKEY_USERS,
    str: "HKU",
};
pub const HKCC: Hive = Hive {
    hkey: windows::Win32::System::Registry::HKEY_CURRENT_CONFIG,
    str: "HKCC",
};

use windows::Win32::{Foundation::ERROR_SUCCESS, System::Registry::*};

fn get_all_registry_rows(hkey: HKEY, rows: &mut Vec<RegistryRow>) -> Result<()> {
    use windows::Win32::System::Registry::*;
    rows.clear();

    // Add ".." entry
    rows.push(RegistryRow {
        name: "..".into(),
        value_display: "".into(),
        value_kind: RegOrStr::RegType2(Additionalreg::Up),
    });

    //let count = query_subkey_count(hkey);
    let mut index = 0;

    // Enumerate subkeys
    let mut name_buf = vec![0u16; 0];
    loop {
        let status = common::regedit::reg_enum_key(hkey, index, &mut name_buf);
        if status == windows::Win32::Foundation::ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != ERROR_SUCCESS {
            return win32_error_to_boxed(status);
        }

        let name = String::from_utf16_lossy(&name_buf);

        rows.push(RegistryRow {
            name,
            value_display: "".into(),
            value_kind: RegOrStr::RegType2(Additionalreg::Key),
        });

        index += 1;
    }

    // Enumerate values
    let mut index = 0;
    let mut name_buf = vec![0u16; 0];
    let mut data_buf = vec![0u8; 0];
    loop {
        let mut data_type = REG_VALUE_TYPE(0);
        let status = common::regedit::reg_enum_value(
            hkey,
            index,
            &mut name_buf,
            &mut data_buf,
            &mut data_type,
        );

        if status == windows::Win32::Foundation::ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != ERROR_SUCCESS {
            return win32_error_to_boxed(status);
        }

        let name = String::from_utf16_lossy(&name_buf);

        let display_value = match data_type {
            REG_BINARY => format!("{} bytes", data_buf.len()),
            REG_DWORD => match data_buf.len() {
                4 => data_buf[..data_buf.len() as usize]
                    .try_into()
                    .map(|b: [u8; 4]| format!("0x{:08X}", u32::from_le_bytes(b)))?,
                _ => format!("invalid dword: {} bytes", data_buf.len()),
            },
            REG_DWORD_BIG_ENDIAN => match data_buf.len() {
                4 => data_buf[..data_buf.len() as usize]
                    .try_into()
                    .map(|b: [u8; 4]| format!("0x{:08X}", u32::from_be_bytes(b)))?,
                _ => format!("invalid big endian dword: {} bytes", data_buf.len()),
            },
            REG_QWORD => match data_buf.len() {
                8 => data_buf[..data_buf.len() as usize]
                    .try_into()
                    .map(|b: [u8; 8]| format!("0x{:016X}", u64::from_le_bytes(b)))?,
                _ => format!("invalid qword: {} bytes", data_buf.len()),
            },
            REG_SZ | REG_EXPAND_SZ | REG_MULTI_SZ => {
                let utf16_vec: Vec<u16> = data_buf[..data_buf.len() as usize]
                    .chunks_exact(2)
                    .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();
                let mut out = String::from_utf16_lossy(&utf16_vec);
                if out.ends_with('\0') {
                    // remove trailing \0, which is normally present
                    out.pop();
                }
                if data_type == REG_MULTI_SZ {
                    if out.ends_with('\0') {
                        // REG_MULTI_SZ has normall two trailing \0
                        out.pop();
                    }
                    out = out.replace('\0', " "); // otherwise embedded \0 has width=0, space is better fo visualizing
                }
                out
            }
            REG_NONE => {
                if data_buf.len() == 0 {
                    "".to_string()
                } else {
                    format!("<unable to parse none value>, {} bytes", data_buf.len())
                }
            }
            _ => format!("<unable to parse value>, {} bytes", data_buf.len()),
        };

        rows.push(RegistryRow {
            name,
            value_display: display_value,
            value_kind: RegOrStr::RegType(data_type),
        });

        index += 1;
    }

    Ok(())
}

fn edit_registry_value(
    old: &mut common::ui::ScreenBuffer,
    new: &mut common::ui::ScreenBuffer,
    key: HKEY,
    display_value: &str,
    value_name: &str,
    value_kind: REG_VALUE_TYPE,
    status: &mut String,
) -> Result<bool> {
    match value_kind {
        REG_SZ | REG_EXPAND_SZ => {}
        REG_QWORD | REG_DWORD | REG_DWORD_BIG_ENDIAN => {}
        REG_MULTI_SZ => {
            status.push_str("editing multiline strings is not supported yet");
            //    return Ok(false);
        }
        REG_BINARY | REG_NONE => {
            status.push_str("editing binary is not supported yet");
            //    return Ok(false);
        }
        _ => {
            status.push_str("editing ??? is not supported yet");
            return Ok(false);
        }
    }

    let new_value = match common::ui::inline_editor(
        old,
        new,
        10, // row
        &format!("Insert value for '{}'", value_name),
        "> ",
        display_value,
    )? {
        Some(v) => v,
        None => {
            status.push_str("Cancelled");
            return Ok(false);
        }
    };

    let name_w: Vec<u16> = value_name.encode_utf16().chain(Some(0)).collect();
    if let Err(err) = common::regedit::set_registry_value(key, &name_w, value_kind, &new_value) {
        write!(status, "{}", err)?;
        return Ok(false);
    }

    return Ok(true);
}

fn goto(
    new_hive: Hive,
    current_hive: &mut Hive,
    current_hkey: &mut HKEY,
    new_path: &mut String,
    current_path: &mut String,
    writable: bool,
    statusbar: &mut String,
    list: &mut List,
) {
    match common::regedit::open_registry_key(new_hive.hkey, &new_path, writable) {
        Ok(new_key) => {
            if let Err(err) = win32_error_to_boxed(unsafe { RegCloseKey(*current_hkey) }) {
                write!(statusbar, "Unable to close old key : {}", err).unwrap();
            }
            *current_hive = new_hive.clone();
            *current_hkey = new_key;
            *current_path = std::mem::take(new_path);
            list.loadkey(*current_hkey, statusbar, true);
        }
        Err(err) => {
            write!(
                statusbar,
                "Unable to open : {}\\{} - {}",
                new_hive.str, new_path, err
            )
            .unwrap();
        }
    }
}
