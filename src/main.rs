extern crate termios;
extern crate termsize;

use std::io;
use std::io::{Read, Write, Result};
use std::io::{BufReader, BufRead};

use std::fs::File;
use std::path::Path;

use termios::*;
use std::str;

const TAB_STOP: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(u8),
    Left,
    Right,
    Up,
    Down,
    Del,
    Home,
    End,
    PageUp,
    PageDown,
}

#[inline(always)]
fn ctrl_key(key: u8) -> Key {
    Key::Char(key & 0x1f)
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
    tsize: termsize::Size,
    cx: usize,
    cy: usize,
    rx: usize,
    rows: Vec<String>,
    rowoff: usize,
    coloff: usize,
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
            tsize: termsize::Size { rows: 25, cols: 80 },
            cx: 0,
            cy: 0,
            rx: 0,
            rows: vec![],
            rowoff: 0,
            coloff: 0,
        }
    }

    pub fn open<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let file = BufReader::new(File::open(path)?);
        self.rows = file.lines().map(|x| x.unwrap()).collect();
        Ok(())
    }

    pub fn init(&mut self) {
       match self.get_window_size() {
           Ok(s) => self.tsize = s,
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

        if c == ctrl_key(b'q') {
            self.exit(0);
        }
        match c {
            Key::Up | Key::Down | Key::Left | Key::Right => self.move_cursor(c),
            Key::PageUp | Key::PageDown => {
                if c == Key::PageUp {
                    self.cy = self.rowoff;
                } else {
                    self.cy = self.rowoff + self.tsize.rows as usize - 1;
                    if self.cy > self.rows.len() {
                        self.cy = self.rows.len();
                    }
                }
                for _ in 0..self.tsize.rows {
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
            _ => {}
        }
        Ok(())
    }

    pub fn refresh_screen(&mut self) -> Result<()> {
        self.scroll();
        self.write("\x1b[?25l\x1b[H")?;
        self.draw_rows()?;
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
        for y in 0..self.tsize.rows {
            let fileoff = y as usize + self.rowoff;
            if fileoff >= self.rows.len() {
                if self.rows.is_empty() && y == self.tsize.rows / 3 {
                    let welcome = format!("Kilo editor -- version {}", env!("CARGO_PKG_VERSION"));
                    let mut padding = (self.tsize.cols as usize - welcome.len()) / 2;
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
                    if line.len() > self.tsize.cols as usize {
                        line = &line[..self.tsize.cols as usize];
                    }
                    s += &line;
                }
            }
            s += "\x1b[K";
            if y < self.tsize.rows - 1 {
                s += "\r\n";
            }
        }
        self.write(s.as_str())?;
        Ok(())
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

        if self.cy >= self.rowoff + self.tsize.rows as usize {
            self.rowoff = self.cy - self.tsize.rows as usize + 1;
        }

        self.rx = 0;
        if self.cy < self.rows.len() {
            let (cx, line) = (self.cx, &self.rows[self.cy]);
            self.rx = self.cx_to_rx(line, cx);
        }
        if self.rx < self.coloff {
            self.coloff = self.rx;
        }

        if self.rx >= self.coloff + self.tsize.cols as usize {
            self.coloff = self.rx - self.tsize.cols as usize + 1;
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
}

fn main() {
    let mut editor = Editor::new();
    editor.init();
    editor.open("src/main.rs").unwrap();
    loop {
        editor.refresh_screen().unwrap();
        editor.process_key().unwrap();
    }
}
