#![allow(dead_code)]

use std::borrow::Borrow;
use std::fmt::{Debug, Display};
use std::io::{self, Write};
pub use std::process::exit;

use anyhow::anyhow;

use crate::util::style::{highlight_error, highlight_info, highlight_warning};

pub fn print_error(err: anyhow::Error) {
    let count = err
        .chain()
        .map(|err| err.to_string())
        .filter(|err| !err.is_empty())
        .enumerate()
        .map(|(i, err)| {
            if i == 0 {
                eprintln!("{} {}", highlight_error("error:"), err);
            } else {
                eprintln!("{} {}", highlight_error("caused by:"), err);
            }
        })
        .count();

    if count == 0 {
        eprintln!("{} an undefined error occurred", highlight_error("error:"));
    }
}

pub fn print_error_msg<S>(err: S)
where
    S: AsRef<str> + Display + Debug + Sync + Send + 'static,
{
    print_error(anyhow!(err));
}

pub fn print_warning<S>(err: S)
where
    S: AsRef<str> + Display + Debug + Sync + Send + 'static,
{
    eprintln!("{} {}", highlight_warning("warning:"), err);
}

pub fn quit() -> ! {
    exit(0);
}

pub fn quit_error(err: anyhow::Error, hints: impl Borrow<ErrorHints>) -> ! {
    print_error(err);
    hints.borrow().print(false);
    exit(1);
}

pub fn quit_error_msg<S>(err: S, hints: impl Borrow<ErrorHints>) -> !
where
    S: AsRef<str> + Display + Debug + Sync + Send + 'static,
{
    quit_error(anyhow!(err), hints);
}

#[derive(Clone, Builder, Default)]
#[builder(default)]
pub struct ErrorHints {
    info: Vec<String>,
}

impl ErrorHints {
    pub fn print(&self, end_newline: bool) {
        for msg in &self.info {
            eprintln!("{} {}", highlight_info("info:"), msg);
        }

        if end_newline {
            eprintln!();
        }

        let _ = io::stderr().flush();
    }
}

impl ErrorHintsBuilder {
    pub fn add_info(mut self, info: String) -> Self {
        if self.info.is_none() {
            self.info = Some(Vec::new());
        }

        if let Some(ref mut list) = self.info {
            list.push(info);
        }

        self
    }
}
