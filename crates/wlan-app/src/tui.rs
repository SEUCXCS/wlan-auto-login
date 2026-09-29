use crate::app::Form;
use crate::infra::error::Message;
use crate::infra::error::{Result, bail};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::{io, time::Duration};

struct Screen;
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
    }
}

pub fn prompt(
    mut form: Form,
    remember_reason: Option<&str>,
    mut message: Option<String>,
) -> Result<Form> {
    let force_remember = remember_reason.is_some();
    if force_remember {
        form.remember = true;
    }
    enable_raw_mode()?;
    let _screen = Screen;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut focus = if form.credentials.username.is_empty() {
        0
    } else {
        1
    };
    let mut fields = [Rect::default(); 5];
    loop {
        terminal.draw(|frame| {
            render(
                frame,
                &form,
                remember_reason,
                message.as_deref(),
                focus,
                &mut fields,
            )
        })?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let submit = match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let was_submit = key.code == KeyCode::Enter && focus == 4;
                match key.code {
                    KeyCode::Esc => bail!(Message::LoginCancelled),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        bail!(Message::LoginCancelled)
                    }
                    KeyCode::Tab | KeyCode::Down => focus = (focus + 1) % 5,
                    KeyCode::BackTab | KeyCode::Up => focus = (focus + 4) % 5,
                    KeyCode::Left | KeyCode::Right if focus == 3 => {
                        let index = cxxy_wlanlogin_lib::Line::ALL
                            .iter()
                            .position(|line| *line == form.line)
                            .unwrap_or_default();
                        form.line = cxxy_wlanlogin_lib::Line::ALL
                            [(index + if key.code == KeyCode::Left { 2 } else { 1 }) % 3];
                    }
                    KeyCode::Char(' ') | KeyCode::Enter if focus == 2 => {
                        toggle_remember(&mut form, force_remember)
                    }
                    KeyCode::Backspace if focus < 2 => {
                        input(&mut form, focus).pop();
                    }
                    KeyCode::Char(c)
                        if focus < 2
                            && !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        let text = input(&mut form, focus);
                        if text.len() < 1024 && !c.is_control() {
                            text.push(c);
                        }
                    }
                    KeyCode::Enter if focus != 4 => focus = (focus + 1) % 5,
                    KeyCode::Enter => {}
                    _ => continue,
                }
                was_submit
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(event::MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);
                if let Some(index) = fields.iter().position(|field| field.contains(position)) {
                    focus = index;
                    if focus == 2 {
                        toggle_remember(&mut form, force_remember);
                    }
                    focus == 4
                } else {
                    false
                }
            }
            _ => false,
        };
        if submit {
            if form.credentials.username.trim().is_empty() || form.credentials.password.is_empty() {
                message = Some("请输入账号和密码".into());
            } else {
                return Ok(form);
            }
        }
    }
}

fn render(
    frame: &mut ratatui::Frame<'_>,
    form: &Form,
    remember_reason: Option<&str>,
    message: Option<&str>,
    focus: usize,
    fields: &mut [Rect; 5],
) {
    let area = frame.area();
    if area.width < 44 || area.height < 22 {
        *fields = [Rect::default(); 5];
        frame.render_widget(Paragraph::new("请将终端调整至至少 44 × 22；Esc 取消"), area);
        return;
    }
    let width = area.width.min(76);
    let content = Rect::new((area.width - width) / 2, 1, width, area.height - 2);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(3),
    ])
    .split(content);
    frame.render_widget(
        Paragraph::new("Student_CX 校园网登录\nTab 切换 · 空格勾选 · Esc 取消"),
        rows[0],
    );
    let password = "•".repeat(form.credentials.password.chars().count());
    let remember = if form.remember {
        "[x] Remember me"
    } else {
        "[ ] Remember me"
    };
    let values = [
        form.credentials.username.as_str(),
        password.as_str(),
        remember,
        form.line.label(),
        "登录",
    ];
    let titles = ["账号", "密码", "保存凭据", "线路（← / →）", "Enter 提交"];
    for i in 0..5 {
        fields[i] = rows[i + 1];
        let style = if focus == i {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let text_width = ratatui::text::Line::from(values[i]).width();
        let inner_width = fields[i].width.saturating_sub(3) as usize;
        let scroll = if i < 2 {
            text_width.saturating_sub(inner_width) as u16
        } else {
            0
        };
        frame.render_widget(
            Paragraph::new(values[i]).scroll((0, scroll)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(titles[i])
                    .border_style(style),
            ),
            fields[i],
        );
        if focus == i && i < 2 {
            frame.set_cursor_position(Position::new(
                fields[i].x + 1 + (text_width.min(inner_width) as u16),
                fields[i].y + 1,
            ));
        }
    }
    let mut notice = if let Some(reason) = remember_reason {
        format!("⚠ {reason}\nRemember me 已开启，无法取消。\n")
    } else {
        String::new()
    };
    if let Some(message) = message {
        notice.push_str(message);
    }
    frame.render_widget(
        Paragraph::new(notice)
            .style(Style::default().fg(Color::Yellow))
            .wrap(Wrap { trim: false }),
        rows[6],
    );
}

fn input(form: &mut Form, focus: usize) -> &mut String {
    if focus == 0 {
        &mut form.credentials.username
    } else {
        &mut form.credentials.password
    }
}

fn toggle_remember(form: &mut Form, forced: bool) {
    if !forced {
        form.remember = !form.remember;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_remember_warning_is_visible_yellow_and_password_is_masked() -> Result<()> {
        for (reason, first_char) in [
            ("安装服务必须保存账号和密码。", "安"),
            ("离线配置仅保存账号和密码。", "离"),
        ] {
            for width in [44, 76, 120] {
                let backend = ratatui::backend::TestBackend::new(width, 22);
                let mut terminal = Terminal::new(backend).unwrap_or_else(|never| match never {});
                let form = Form {
                    credentials: cxxy_wlanlogin_lib::Credentials {
                        username: "student".into(),
                        password: "never-echo-this".into(),
                    },
                    line: Default::default(),
                    remember: true,
                };
                terminal
                    .draw(|frame| {
                        render(
                            frame,
                            &form,
                            Some(reason),
                            None,
                            2,
                            &mut [Rect::default(); 5],
                        )
                    })
                    .unwrap_or_else(|never| match never {});
                let cells = &terminal.backend().buffer().content;
                let visible = cells
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .replace(' ', "");
                assert!(visible.contains(reason));
                assert!(visible.contains("Rememberme已开启，无法取消"));
                assert!(visible.contains("[x]Rememberme"));
                assert!(!visible.contains("never-echo-this"));
                assert!(
                    cells
                        .iter()
                        .any(|cell| cell.symbol() == first_char && cell.fg == Color::Yellow)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn required_remember_cannot_be_unchecked() {
        let mut form = Form {
            credentials: cxxy_wlanlogin_lib::Credentials {
                username: String::new(),
                password: String::new(),
            },
            line: Default::default(),
            remember: true,
        };
        toggle_remember(&mut form, true);
        assert!(form.remember);
        toggle_remember(&mut form, false);
        assert!(!form.remember);
    }
}
