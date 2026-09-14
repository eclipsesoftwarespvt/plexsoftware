#![allow(dead_code)]

use colored::{ColoredString, Colorize};

pub fn highlight(msg: &str) -> ColoredString {
    msg.yellow()
}

pub fn highlight_error(msg: &str) -> ColoredString {
    msg.red().bold()
}

pub fn highlight_warning(msg: &str) -> ColoredString {
    highlight(msg).bold()
}

pub fn highlight_info(msg: &str) -> ColoredString {
    msg.cyan()
}
