use std::ffi::OsString;
use std::process;

use yashik_windows_launcher::{run, Outcome, SystemWsl};

fn main() {
    let arguments = std::env::args_os().skip(1).collect::<Vec<OsString>>();
    let mut wsl = SystemWsl;
    let code = match run(&arguments, &mut wsl) {
        Ok(Outcome::Help) => {
            print!("{}", yashik_windows_launcher::help_text());
            0
        }
        Ok(Outcome::Version) => {
            println!("yashik {}", yashik_windows_launcher::VERSION);
            0
        }
        Ok(Outcome::Exit(code)) => code,
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    };
    process::exit(code);
}
