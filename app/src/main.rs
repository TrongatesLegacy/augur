// Release builds on Windows are GUI apps (no console window); the command line attaches to the parent console.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod cli;
mod secure;
mod settings;
mod views;

use clap::Parser;

fn main() {
    // with a command it's the command line; without one, the app
    if std::env::args_os().len() > 1 {
        #[cfg(windows)]
        #[allow(unsafe_code)] // a Windows system call: print to the console the command was run from
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
        }
        let cli = cli::Cli::parse();
        if let Err(e) = cli::run(cli) {
            eprintln!("augur: {e}");
            std::process::exit(1);
        }
        return;
    }
    app::run();
}
#[cfg(test)]
mod ui_tests;
