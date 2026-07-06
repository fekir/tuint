pub struct ScreenBuffer {
    pub width: usize,
    pub height: usize,
    cells: Vec<u8>, // row-major: y * width + x
}

impl ScreenBuffer {
    pub fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            cells: vec![],
        }
    }

    pub fn set_line(&mut self, y: usize, text: &str) {
        if y >= self.height {
            return;
        }
        let start = y * self.width;
        let row = &mut self.cells[start..start + self.width];
        row.fill(b' ');
        let bytes = text.as_bytes();
        let n = bytes.len().min(self.width);
        row[..n].copy_from_slice(&bytes[..n]);
    }

    pub fn row(&self, y: usize) -> &[u8] {
        let start = y * self.width;
        &self.cells[start..start + self.width]
    }

    pub fn row_mut(&mut self, y: usize) -> &mut [u8] {
        let start = y * self.width;
        &mut self.cells[start..start + self.width]
    }
}

pub fn resize_buffers(
    old: &mut ScreenBuffer,
    new: &mut ScreenBuffer,
    width: usize,
    height: usize,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    old.width = width;
    old.height = height;
    old.cells.resize(width * height, b' ');
    old.cells.fill(b' ');

    new.width = width;
    new.height = height;
    new.cells.resize(width * height, b' ');
    new.cells.fill(b' ');

    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
    )?;
    Ok(())
}

pub fn print_diff(
    old: &mut ScreenBuffer,
    new: &ScreenBuffer,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut stdout = std::io::stdout();

    for y in 0..new.height {
        let row_old = old.row_mut(y);
        let row_new = new.row(y);

        if row_old == row_new {
            continue;
        }

        let mut x = 0;

        while x < new.width {
            while x < new.width && row_old[x] == row_new[x] {
                x += 1;
            }

            let start = x;
            while x < new.width && row_old[x] != row_new[x] {
                x += 1;
            }
            let length = x - start;

            crossterm::QueueableCommand::queue(
                &mut stdout,
                crossterm::cursor::MoveTo(start as u16, y as u16),
            )?;

            //  write entire region in one call
            let slice = &row_new[start..start + length];
            std::io::Write::write_all(&mut stdout, slice)?;

            row_old[start..start + length].copy_from_slice(slice);
        }
    }

    std::io::Write::flush(&mut stdout)?;
    Ok(())
}

pub fn choose_dialog(
    old: &mut ScreenBuffer,
    new: &mut ScreenBuffer,
    message: &str,
    buttons: &[&str],
) -> std::result::Result<Option<usize>, Box<dyn std::error::Error>> {
    assert!(buttons.len() > 0);

    // see how btn_line is rendered
    let max_width = buttons
        .iter()
        .map(|b| 5 + b.len())
        .sum::<usize>()
        .max(message.len())
        + 4;
    let height = 7;

    let mut sel = 0;
    let mut last_width = 0;
    let mut last_height = 0;
    let mut needs_redraw = true;

    loop {
        if needs_redraw {
            let top = (new.height - height) / 2;
            if last_width != new.width || last_height != new.height {
                last_width = new.width;
                last_height = new.height;

                //let left = (new.width - width) / 2;

                // Draw static frame
                new.set_line(top + 0, &format!("/{0}\\", "~".repeat(max_width - 2)));
                new.set_line(top + 1, &format!("|{0}|", " ".repeat(max_width - 2)));

                let pad1 = (max_width - message.len()) / 2 - 1;
                let pad2 = max_width - message.len() - pad1 - 2;
                new.set_line(
                    top + 2,
                    &format!("|{0}{message}{1}|", " ".repeat(pad1), " ".repeat(pad2)),
                );

                new.set_line(top + 3, &format!("|{0}|", " ".repeat(max_width - 2)));
                new.set_line(top + 5, &format!("|{0}|", " ".repeat(max_width - 2)));
                new.set_line(top + 6, &format!("\\{0}/", "~".repeat(max_width - 2)));
            }
            // Build button line with selection
            let mut btn_line = String::new();
            for (i, b) in buttons.iter().enumerate() {
                if i == sel {
                    btn_line.push_str(&format!("[ {b} ] "));
                } else {
                    btn_line.push_str(&format!("  {b}   "));
                }
            }

            // Center buttons
            let pad = (max_width - btn_line.len()) / 2 - 1;
            new.set_line(top + 4, &format!("|{0}{btn_line}{0}|", " ".repeat(pad)));

            print_diff(old, new)?;
            needs_redraw = false;
        }

        // Read key
        match crossterm::event::read()? {
            crossterm::event::Event::Key(key)
                if key.kind == crossterm::event::KeyEventKind::Press =>
            {
                needs_redraw = true;
                match key.code {
                    crossterm::event::KeyCode::Left => {
                        sel = (sel + buttons.len() - 1) % buttons.len();
                    }
                    crossterm::event::KeyCode::Right => {
                        sel = (sel + 1) % buttons.len();
                    }
                    crossterm::event::KeyCode::Enter => {
                        return Ok(Some(sel));
                    }
                    crossterm::event::KeyCode::Esc
                    | crossterm::event::KeyCode::F(10)
                    | crossterm::event::KeyCode::Char('q') => {
                        return Ok(None);
                    }
                    _ => {}
                }
            }
            crossterm::event::Event::Resize(width, height) => {
                if new.width != width.into() || new.height != height.into() {
                    resize_buffers(old, new, width.into(), height.into())?;
                    needs_redraw = true;
                }
            }
            _ => {}
        }
    }
}

pub fn info_dialog(
    old: &mut ScreenBuffer,
    new: &mut ScreenBuffer,
    message: &str,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    resize_buffers(old, new, old.width, new.height)?;

    let mut needs_redraw = true;

    loop {
        if needs_redraw {
            let mut index = 0;
            for s in message.lines() {
                new.set_line(index, s);
                index += 1;
            }
            print_diff(old, new)?;
            needs_redraw = false;
        }
        // Read key
        match crossterm::event::read()? {
            crossterm::event::Event::Key(key)
                if key.kind == crossterm::event::KeyEventKind::Press =>
            {
                match key.code {
                    crossterm::event::KeyCode::Esc
                    | crossterm::event::KeyCode::F(10)
                    | crossterm::event::KeyCode::Char('q') => {
                        return Ok(());
                    }
                    _ => {}
                }
            }

            crossterm::event::Event::Resize(width, height) => {
                if new.width != width.into() || new.height != height.into() {
                    resize_buffers(old, new, width.into(), height.into())?;
                    needs_redraw = true;
                }
            }
            _ => {}
        }
    }
}

pub fn inline_editor(
    old: &mut ScreenBuffer,
    new: &mut ScreenBuffer,
    row: usize,
    description: &str,
    prompt: &str,
    input: &str,
) -> std::result::Result<Option<String>, Box<dyn std::error::Error>> {
    let mut stdout = std::io::stdout();

    let mut input: Vec<char> = input.chars().collect();
    let mut cursor = input.len();

    let mut line = format!("{prompt}{}", input.iter().collect::<String>());
    let central_row = row + 3;
    new.set_line(central_row, &line);

    let mut last_width = 0;
    let mut last_height = 0;
    let mut needs_redraw = true;

    loop {
        if needs_redraw {
            if last_width != new.width || last_height != new.height {
                last_width = new.width;
                last_height = new.height;

                new.set_line(row + 0, "");
                new.set_line(row + 1, &"-".repeat(new.width));
                new.set_line(row + 2, description);
                new.set_line(row + 4, &"-".repeat(new.width));
                new.set_line(row + 5, "");
            }
            print_diff(old, new)?;
            crossterm::QueueableCommand::queue(&mut stdout, crossterm::cursor::Show)?;
            crossterm::QueueableCommand::queue(
                &mut stdout,
                crossterm::cursor::MoveTo((prompt.len() + cursor) as u16, central_row as u16),
            )?;
            std::io::Write::flush(&mut stdout)?;
            // avoid cursor flickering when modifying text
            // and hide cursor if exiting
            crossterm::QueueableCommand::queue(&mut stdout, crossterm::cursor::Hide)?;
            needs_redraw = false;
        }
        match crossterm::event::read()? {
            crossterm::event::Event::Key(key)
                if key.kind == crossterm::event::KeyEventKind::Press =>
            {
                needs_redraw = true;
                match key.code {
                    crossterm::event::KeyCode::Enter => {
                        return Ok(Some(input.iter().collect()));
                    }
                    crossterm::event::KeyCode::Esc
                    | crossterm::event::KeyCode::F(10)
                    | crossterm::event::KeyCode::Char('q') => {
                        return Ok(None);
                    }
                    crossterm::event::KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                    }
                    crossterm::event::KeyCode::Right => {
                        cursor = (cursor + 1).min(input.len());
                    }
                    crossterm::event::KeyCode::Home => cursor = 0,
                    crossterm::event::KeyCode::End => cursor = input.len(),
                    crossterm::event::KeyCode::Backspace => {
                        if cursor > 0 {
                            cursor -= 1;
                            input.remove(cursor);
                        }
                    }
                    crossterm::event::KeyCode::Delete => {
                        if cursor < input.len() {
                            input.remove(cursor);
                        }
                    }
                    crossterm::event::KeyCode::Char(c) => {
                        input.insert(cursor, c);
                        cursor += 1;
                    }

                    _ => {}
                }
            }

            crossterm::event::Event::Resize(width, height) => {
                if new.width != width.into() || new.height != height.into() {
                    resize_buffers(old, new, width.into(), height.into())?;
                    needs_redraw = true;
                }
            }

            _ => {}
        }
        line.clear();
        line.push_str(prompt);
        line.extend(input.iter().copied());
        new.set_line(central_row, &line);
    }
}

pub(crate) fn trim(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let end = (0..=max - 3).rfind(|m| s.is_char_boundary(*m)).unwrap();
    format!("{}...", &s[..end])
}
