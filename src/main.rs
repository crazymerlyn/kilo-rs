extern crate termios;
extern crate termsize;

use std::io;
use std::io::{Read, Write, Result};

use termios::*;
use std::str;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(u8),
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
}

#[inline(always)]
fn ctrl_key(key: u8) -> Key {
    Key::Char(key & 0x1f)
}

pub struct Editor {
    term: Termios,
    stdin: io::Stdin,
    stdout: io::Stdout,
    tsize: termsize::Size,
    cx: u16,
    cy: u16,
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
            cx: 10,
            cy: 0,
        }
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

            if s[0] != b'[' {
                return Ok(Key::Char(b'\x1b'));
            }

            match s[1] {
                b'A' => return Ok(Key::Up),
                b'B' => return Ok(Key::Down),
                b'C' => return Ok(Key::Right),
                b'D' => return Ok(Key::Left),
                _ => return Ok(Key::Char(b'\x1b')),
            }
        }

        Ok(Key::Char(buf[0]))
    }

    fn move_cursor(&mut self, key: Key) {
        match key {
            Key::Left => {
                if self.cx > 0 {
                    self.cx -= 1;
                }
            }
            Key::Right => {
                if self.cx < self.tsize.cols - 1 {
                    self.cx += 1;
                }
            }
            Key::Up => {
                if self.cy > 0 {
                    self.cy -= 1;
                }
            }
            Key::Down => {
                if self.cy < self.tsize.rows - 1 {
                    self.cy += 1;
                }
            }
            _ => {}
        }
    }

    pub fn process_key(&mut self) -> Result<()> {
        let c = self.read_key()?;

        if c == ctrl_key(b'q') {
            self.exit(0);
        }
        match c {
            Key::Up | Key::Down | Key::Left | Key::Right => self.move_cursor(c),
            _ => {}
        }
        Ok(())
    }

    pub fn refresh_screen(&mut self) -> Result<()> {
        self.write("\x1b[?25l\x1b[H")?;
        self.draw_rows()?;
        let command = format!("\x1b[{};{}H", self.cy + 1, self.cx + 1);
        self.write(command)?;
        self.write("\x1b[?25h")?;
        Ok(())
    }

    pub fn draw_rows(&mut self) -> Result<()> {
        let mut s = "".to_string();
        for y in 0..self.tsize.rows {
            if y == self.tsize.rows / 3 {
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
}

fn main() {
    let mut editor = Editor::new();
    editor.init();
    loop {
        editor.refresh_screen().unwrap();
        editor.process_key().unwrap();
    }
}
