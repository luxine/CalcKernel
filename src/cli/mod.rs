mod args;
#[cfg(feature = "native-toolchain")]
mod cache;
mod commands;
mod output;
#[cfg(feature = "native-toolchain")]
mod run;

use args::usage;
use commands::{dispatch, run_version};

pub(crate) fn run(args: Vec<String>) -> i32 {
    let Some(command) = args.first().map(String::as_str) else {
        eprint!("{}", usage());
        return 2;
    };

    #[cfg(feature = "native-toolchain")]
    if command == "__ckc-run-child" {
        return run::run_private_child(&args[1..]);
    }

    #[cfg(feature = "native-toolchain")]
    if command == "run" {
        return run::run_public_parent(&args[1..]);
    }

    if command == "--help" || command == "-h" {
        print!("{}", usage());
        return 0;
    }

    if command == "--version" || command == "-V" {
        return match run_version(&args[1..]) {
            Ok(()) => 0,
            Err(message) => {
                print_error(&message);
                1
            }
        };
    }

    let Some(result) = dispatch(command, &args[1..]) else {
        eprint!("{}", usage());
        return 2;
    };

    match result {
        Ok(()) => 0,
        Err(message) => {
            print_error(&message);
            1
        }
    }
}

fn print_error(message: &str) {
    if message.ends_with('\n') {
        eprint!("{message}");
    } else {
        eprintln!("{message}");
    }
}
