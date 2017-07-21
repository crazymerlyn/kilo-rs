extern crate termios;
extern crate termsize;

use std::io;
use std::io::{Read, Write, Result};
use std::io::{BufReader, BufRead};

use std::fs::File;
use std::path::Path;

use termios::*;
use std::str;
use std::time::{Instant, Duration};
use std::ops::Sub;

use std::error::Error;

const TAB_STOP: usize = 8;
const QUIT_TIMES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(u8),
    Ctrl(u8),
    Left,
    Right,
    Up,
    Down,
    Del,
    Home,
    End,
    PageUp,
    PageDown,
    Return,
    Backspace,
}

trait Render {
    fn render(&self) -> Self;
}

impl Render for String {
    fn render(&self) -> String {
        let mut res = "".to_string();

        for ch in self.chars() {
            if ch == '\t' {
                res.push(' ');
                while res.len() % TAB_STOP != 0 { res.push(' '); };
            } else {
                res.push(ch);
            }
        }
        res
    }
}

pub struct Editor {
    term: Termios,
    stdin: io::Stdin,
    stdout: io::Stdout,
    numrows: usize,
    numcols: usize,
    cx: usize,
    cy: usize,
    rx: usize,
    rows: Vec<String>,
    rowoff: usize,
    coloff: usize,
    dirty: bool,
    quit_times: usize,
    filename: Option<String>,
    status_msg: String,
    status_msg_time: Instant,
}

impl Editor {
    pub fn new() -> Editor {
        let mut term = Termios::from_fd(0).expect("Failed to get termios");
        let original = term;

        term.c_iflag &= !(BRKINT | ICRNL | INPCK | ISTRIP | IXON);
        term.c_oflag &= !(OPOST);
        term.c_cflag |= CS8;
        term.c_lflag &= !(ECHO | IEXTEN | ICANON | ISIG);

        term.c_cc[VMIN] = 0;
        term.c_cc[VTIME] = 1;

        tcsetattr(0, TCSAFLUSH, &mut term).expect("Failed to get raw mode");

        Editor { 
            term: original,
            stdin: io::stdin(),
            stdout: io::stdout(),
            numrows: 25,
            numcols: 80,
            cx: 0,
            cy: 0,
            rx: 0,
            rows: vec![],
            rowoff: 0,
            coloff: 0,
            dirty: false,
            quit_times: QUIT_TIMES,
            filename: None,
            status_msg: "".to_string(),
            status_msg_time: Instant::now().sub(Duration::from_secs(100)),
        }
    }

    pub fn open<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let file = BufReader::new(File::open(path.as_ref())?);
        self.filename = path.as_ref().to_str().map(|x| x.to_string());
        self.rows = file.lines().map(|x| x.unwrap()).collect();
        self.dirty = false;
        Ok(())
    }

    pub fn init(&mut self) {
       match self.get_window_size() {
           Ok(s) => {
               self.numcols = s.cols as usize;
               self.numrows = s.rows as usize - 2;
           }
           _ => self.die("Failed to get window size")
       }
    }

    pub fn read_key(&mut self) -> Result<Key> {
        let mut buf = [0; 1];
        while self.stdin.read(&mut buf)? == 0 {}

        if buf[0] == b'\x1b' {
            let mut s = [0;3];
            match self.read_char() {
                Some(c) => s[0] = c,
                _ => return Ok(Key::Char(b'\x1b'))
            }

            match self.read_char() {
                Some(c) => s[1] = c,
                _ => return Ok(Key::Char(b'\x1b'))
            }

            if s[0] == b'[' {
                if s[1] >= b'0' && s[1] <= b'9' {
                    match self.read_char() {
                        Some(c) => s[2] = c,
                        _ => return Ok(Key::Char(b'\x1b'))
                    }
                    if s[2] == b'~' {
                        match s[1] {
                            b'1' | b'7' => return Ok(Key::Home),
                            b'2' | b'8' => return Ok(Key::End),
                            b'3' => return Ok(Key::Del),
                            b'5' => return Ok(Key::PageUp),
                            b'6' => return Ok(Key::PageDown),
                            _ => return Ok(Key::Char(b'\x1b'))
                        }
                    }
                } else {
                    match s[1] {
                        b'A' => return Ok(Key::Up),
                        b'B' => return Ok(Key::Down),
                        b'C' => return Ok(Key::Right),
                        b'D' => return Ok(Key::Left),
                        b'H' => return Ok(Key::Home),
                        b'F' => return Ok(Key::End),
                        _ => return Ok(Key::Char(b'\x1b')),
                    }
                }
            } else if s[0] == b'O' {
                match s[1] {
                    b'H' => return Ok(Key::Home),
                    b'F' => return Ok(Key::End),
                    _ => {}
                }
            }
        }

        if buf[0] == 127 {
            return Ok(Key::Backspace);
        }

        if buf[0] & 0x1f == buf[0] {
            return Ok(Key::Ctrl(buf[0] | 0x60));
        }

        Ok(Key::Char(buf[0]))
    }

    fn move_cursor(&mut self, key: Key) {
        match key {
            Key::Left => {
                if self.cx > 0 {
                    self.cx -= 1;
                } else if self.cy > 0 {
                    self.cy -= 1;
                    self.cx = self.rows[self.cy].len();
                }
            }
            Key::Right => {
                if self.cy < self.rows.len() {
                    if self.cx < self.rows[self.cy].len() {
                        self.cx += 1;
                    } else {
                        self.cy += 1;
                        self.cx = 0;
                    }
                }
            }
            Key::Up => {
                if self.cy > 0 {
                    self.cy -= 1;
                }
            }
            Key::Down => {
                if self.cy < self.rows.len() {
                       self.cy += 1;
                }
            }
            _ => {}
        };

        let rowlen = if self.rows.len() > self.cy {
            self.rows[self.cy].len()
        } else {
            0
        };
        if self.cx > rowlen {
            self.cx = rowlen;
        };
    }

    pub fn process_key(&mut self) -> Result<()> {
        let c = self.read_key()?;

        if c == Key::Ctrl(b'q') {
            if self.dirty && self.quit_times > 0 {
                let s = format!("WARNING!!! File has unsaved changes. Press Ctrl-Q {} more times to quit", self.quit_times);
                self.set_status_msg(s);
                self.quit_times -= 1;
                return Ok(());
            }
            self.exit(0);
        }
        match c {
            Key::Up | Key::Down | Key::Left | Key::Right => self.move_cursor(c),
            Key::PageUp | Key::PageDown => {
                if c == Key::PageUp {
                    self.cy = self.rowoff;
                } else {
                    self.cy = self.rowoff + self.numrows - 1;
                    if self.cy > self.rows.len() {
                        self.cy = self.rows.len();
                    }
                }
                for _ in 0..self.numrows {
                    self.move_cursor(if c == Key::PageUp { Key::Up } else { Key::Down });
                }
            }
            Key::Home => self.cx = 0,
            Key::End  => {
                if self.cy < self.rows.len() {
                    self.cx = self.rows[self.cy].len();
                } else {
                    self.cx = 0;
                }
            }
            Key::Ctrl(b's') => match self.save() {
                Ok(n) => self.set_status_msg(
                    format!("{} bytes written to disk", n)
                    ),
                Err(e) => self.set_status_msg(
                    format!("Can't save! I/O error: {}", e.description())
                    ),
            },
            Key::Char(b'\r') => { /* TODO */ },
            Key::Backspace | Key::Del | Key::Ctrl(b'h') => {
                if c == Key::Del { self.move_cursor(Key::Right); };
                self.del_char();
            },
            Key::Ctrl(b'l') | Key::Char(b'\x1b') => {},
            Key::Char(c) => self.insert_char(c as char),
            _ => {}
        }
        self.quit_times = QUIT_TIMES;
        Ok(())
    }

    pub fn refresh_screen(&mut self) -> Result<()> {
        self.scroll();
        self.write("\x1b[?25l\x1b[H")?;
        self.draw_rows()?;
        self.draw_status_bar()?;
        self.draw_message_bar()?;
        let command = format!(
            "\x1b[{};{}H",
            self.cy - self.rowoff + 1,
            self.rx - self.coloff + 1);
        self.write(command)?;
        self.write("\x1b[?25h")?;
        Ok(())
    }

    pub fn draw_rows(&mut self) -> Result<()> {
        let mut s = "".to_string();
        for y in 0..self.numrows {
            let fileoff = y + self.rowoff;
            if fileoff >= self.rows.len() {
                if self.rows.is_empty() && y == self.numrows / 3 {
                    let welcome = format!("Kilo editor -- version {}", env!("CARGO_PKG_VERSION"));
                    let mut padding = (self.numcols - welcome.len()) / 2;
                    if padding > 0 {
                        s += "~";
                        padding -= 1;
                    }
                    for _ in 0..padding {
                        s += " ";
                    }
                    s += welcome.as_str();
                } else {
                    s += "~";
                }
            } else {
                let row = self.rows[fileoff].render();
                if self.coloff < row.len() {
                    let mut line = &row[self.coloff..];
                    if line.len() > self.numcols {
                        line = &line[..self.numcols];
                    }
                    s += &line;
                }
            }
            s += "\x1b[K";
            s += "\r\n";
        }
        self.write(s.as_str())?;
        Ok(())
    }

    fn draw_status_bar(&mut self) -> Result<()> {
        let mut s = "".to_string();
        s += "\x1b[7m";
        let filedesc = format!(
            "{:.20} - {} lines {}",
            self.filename.as_ref().unwrap_or(&"[No Name]".to_string()),
            self.rows.len(),
            if self.dirty { "(modified)" } else { "" });
        let linedesc = format!("{}/{}", self.cy + 1, self.rows.len());
        let line = if filedesc.len() > self.numcols {
            &filedesc[..self.numcols]
        } else {
            &filedesc
        };
        s += line;

        for i in line.len()..self.numcols {
            if self.numcols - i == linedesc.len() {
                s += &linedesc;
                break;
            } else {
                s.push(' ');
            }
        }
        s += "\x1b[m";
        s += "\r\n";
        self.write(s)?;
        Ok(())
    }

    fn draw_message_bar(&mut self) -> Result<()> {
        let mut res = "".to_string();
        res += "\x1b[K";
        if Instant::now().duration_since(self.status_msg_time).as_secs() < 5 {
            res += if self.status_msg.len() > self.numcols {
                &self.status_msg[..self.numcols]
            } else {
                &self.status_msg
            };
        }
        self.write(&res)?;
        Ok(())
    }

    fn set_status_msg<S: AsRef<str>>(&mut self, message: S) {
        self.status_msg = message.as_ref().to_owned();
        self.status_msg_time = Instant::now();
    }

    fn get_cursor_position(&mut self) -> Result<(u16, u16)> {
        self.write("\x1b[6n")?;
        self.write("\r\n")?;
        
        let mut charbuf = [0; 1];
        let mut buf = [0; 32];
        let mut i = 0;
        while i < buf.len() {
            if self.stdin.read(&mut charbuf)? == 0 {
                break;
            }
            if charbuf[0] == b'R' { break }
            buf[i] = charbuf[0];
            i += 1;
        }
        if buf[0] != b'\x1b' || buf[1] != b'[' {
            return Err(io::Error::new(io::ErrorKind::Other, "Terminal error"));
        }
        let dims : Vec<_> = str::from_utf8(&buf[2..i]).unwrap().split(";").collect();
        if dims.len() != 2 {
            return Err(io::Error::new(io::ErrorKind::Other, "Terminal error"));
        }

        Ok((dims[0].parse().unwrap(), dims[1].parse().unwrap()))
    }

    fn get_window_size(&mut self) -> Result<termsize::Size> {
        match termsize::get() {
            Some(s) => Ok(s),
            _ => {
                self.write("\x1b[999C\x1b[999B")?; 
                let (rows, cols) = self.get_cursor_position()?;
                Ok(termsize::Size { rows, cols })
            }
        }
    }

    fn write<S: AsRef<str>>(&mut self, text: S) -> Result<()> {
        write!(self.stdout, "{}", text.as_ref())?;
        self.stdout.flush()?;
        Ok(())
    }

    fn exit(&mut self, code: i32) {
        self.write("\x1b[2J\x1b[H").unwrap();
        tcsetattr(0, TCSAFLUSH, &mut self.term).expect("Failed to restore state");
        std::process::exit(code);
    }

    fn die(&mut self, message: &str) {
        self.write("\x1b[2J\x1b[H").unwrap();
        write!(io::stderr(), "{}", message).unwrap();
        self.exit(1)
    }

    fn read_char(&mut self) -> Option<u8> {
        let mut b = [0;1];
        let c = self.stdin.read(&mut b).unwrap_or(0);
        if c == 1 {
            Some(b[0])
        } else {
            None
        }
    }

    fn scroll(&mut self) {
        if self.cy < self.rowoff {
            self.rowoff = self.cy;
        }

        if self.cy >= self.rowoff + self.numrows {
            self.rowoff = self.cy - self.numrows + 1;
        }

        self.rx = 0;
        if self.cy < self.rows.len() {
            let (cx, line) = (self.cx, &self.rows[self.cy]);
            self.rx = self.cx_to_rx(line, cx);
        }
        if self.rx < self.coloff {
            self.coloff = self.rx;
        }

        if self.rx >= self.coloff + self.numcols {
            self.coloff = self.rx - self.numcols + 1;
        }
    }

    fn cx_to_rx<S: AsRef<str>>(&self, s: S, cx: usize) -> usize {
        let mut rx = 0;
        for ch in s.as_ref()[..cx].chars() {
            if ch == '\t' {
                rx += TAB_STOP - (rx % TAB_STOP);
            } else {
                rx += 1;
            }
        }
        rx
    }

    fn insert_char(&mut self, c: char) {
        if self.cy == self.rows.len() {
            self.rows.push("".to_string());
        }
        let mut row = &mut self.rows[self.cy];
        if self.cx >= row.len() {
            row.push(c);
        } else {
            *row = row[..self.cx].to_string() + c.to_string().as_str() + &row[self.cx..];
        }

        self.cx += 1;

        self.dirty = true;
    }

    fn del_char(&mut self) {
        if self.cy == self.rows.len() { return; };
        if self.cx == 0 && self.cy == 0 { return; };

        if self.cx > 0 {
            self.cx -= 1;
            let mut row = &mut self.rows[self.cy];
            *row = row[..self.cx].to_string() + &row[self.cx+1..];
        } else {
            self.cx = self.rows[self.cy - 1].len();
            self.rows[self.cy - 1] += &self.rows[self.cy].clone();
            self.rows.remove(self.cy);
            self.cy -= 1;
        }
        self.dirty = true;
    }

    fn rows_to_string(&self) -> String {
        self.rows.join("\n") + "\n"
    }

    pub fn save(&mut self) -> Result<usize> {
        match self.filename {
            Some(ref path) => {
                let mut file = File::create(path)?;
                let res = file.write(self.rows_to_string().as_bytes());
                if let Ok(_) = res {
                    self.dirty = false;
                }
                return res;
            },
            _ => Ok(0),
        }
    }
}

fn main() {
    let mut editor = Editor::new();
    editor.init();
    editor.open("./test.txt").unwrap();

    editor.set_status_msg("HELP: Ctrl-S = save | Ctrl-Q = quit");

    loop {
        editor.refresh_screen().unwrap();
        editor.process_key().unwrap();
    }
}
