//! L-Helper for Linux: performance modes, power limits, 24-zone keyboard lighting, battery and screen for Lenovo
//! LOQ, Legion and IdeaPad laptops. Runs as the signed-in user; see hw/mod.rs for how hardware writes work.
//!
//!   lhelper                 open the window (or bring the running one forward)
//!   lhelper --tray          start in the tray (login autostart)
//!   lhelper --next-lighting next lighting preset in the running L-Helper (for a keyboard shortcut)

mod app;
mod autostart;
// Shared with Windows; Linux uses part of them
#[allow(dead_code)]
#[path = "../../src/config.rs"]
mod config;
mod hw;
mod instance;
mod lighting;
mod log;
#[allow(dead_code)]
#[path = "../../src/logo.rs"]
mod logo;
mod notify;
mod tray;
mod ui;

const HELP: &str = "L-Helper for Lenovo LOQ, Legion and IdeaPad laptops

Usage: lhelper [--tray | --next-lighting | --version]

  --tray           Start in the tray without opening the window
  --next-lighting  Switch the running L-Helper to the next lighting preset
  --version        Print the version";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |a: &str| args.iter().any(|x| x == a);
    if has("--help") || has("-h") {
        println!("{HELP}");
        return;
    }
    if has("--version") {
        println!("L-Helper {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if has("--next-lighting") {
        if !instance::send("next-lighting") {
            eprintln!("L-Helper isn't running");
            std::process::exit(1);
        }
        return;
    }
    let start_hidden = has("--tray");
    // Already running: show that one
    if instance::send(if start_hidden { "noop" } else { "open" }) {
        return;
    }
    if hw::is_root() {
        eprintln!("Run L-Helper as your normal user: it asks for permission itself when it changes hardware settings.");
    }

    log!("Start {} (kernel {})", env!("CARGO_PKG_VERSION"), hw::read("/proc/sys/kernel/osrelease").unwrap_or_default());
    instance::listen(|command| match command {
        "open" => ui::request_open(),
        "next-lighting" => app::cycle_lighting(),
        _ => {}
    });

    lighting::init();
    app::start_background();
    tray::start(start_hidden);
    ui::run(start_hidden);
}
