#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate derive_builder;
#[macro_use]
extern crate log;

pub(crate) mod action;
pub(crate) mod config;
pub(crate) mod forge;
pub(crate) mod join;
#[cfg(feature = "lobby")]
pub(crate) mod lobby;
pub(crate) mod mc;
pub(crate) mod monitor;
pub(crate) mod net;
pub(crate) mod os;
pub(crate) mod probe;
pub(crate) mod proto;
pub(crate) mod proxy;
pub(crate) mod server;
pub(crate) mod service;
pub(crate) mod status;
pub(crate) mod types;
pub(crate) mod util;

use std::env;

#[cfg(all(windows, not(feature = "rcon")))]
compile_error!("Must enable \"rcon\" feature on Windows.");

const LOG_LEVEL: &str = "info";
const BRAND: &str = "[PlexScale]";

fn main() -> Result<(), ()> {
    init_log();
    action::start::invoke(&java_args())
}

fn java_args() -> Vec<String> {
    env::args()
        .skip(1)
        .filter(|arg| {
            let arg = arg.trim();
            !arg.is_empty() && arg != "start" && arg != "run"
        })
        .collect()
}

fn init_log() {
    use std::io::Write;

    use env_logger::fmt::Color;

    env::set_var("RUST_LOG", LOG_LEVEL);

    env_logger::Builder::from_env(env_logger::Env::default())
        .write_style(env_logger::fmt::WriteStyle::Always)
        .format(|buf, record| {
            let mut tag_style = buf.style();
            tag_style.set_color(Color::Cyan).set_bold(true);

            let mut msg_style = buf.style();
            match record.level() {
                log::Level::Error => {
                    msg_style.set_color(Color::Red).set_bold(true);
                }
                log::Level::Warn => {
                    msg_style.set_color(Color::Yellow);
                }
                log::Level::Debug | log::Level::Trace => {
                    msg_style.set_color(Color::Rgb(120, 120, 120));
                }
                log::Level::Info => {
                    msg_style.set_color(Color::White);
                }
            }

            writeln!(
                buf,
                "{} {}",
                tag_style.value(BRAND),
                msg_style.value(record.args()),
            )
        })
        .init();
}
