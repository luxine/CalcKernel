mod args;
#[cfg(feature = "native-toolchain")]
mod cache;
mod commands;
mod lsp;
mod output;
mod pgo;
#[cfg(feature = "native-toolchain")]
mod run;

use args::usage;
use commands::{dispatch, run_version};

pub(crate) fn run(args: Vec<String>) -> i32 {
    let Some(command) = args.first().map(String::as_str) else {
        eprint!("{}", usage());
        return 2;
    };

    let deferred_replay = command == "build"
        && args[1..]
            .iter()
            .any(|arg| arg == "--tune-use" || arg.starts_with("--tune-use="));
    if command == "tune" || deferred_replay {
        print_error("Offline tuning is unavailable in this release.");
        return 1;
    }

    #[cfg(feature = "native-toolchain")]
    if command == "__ckc-run-child" {
        return run::run_private_child(&args[1..]);
    }

    #[cfg(feature = "native-toolchain")]
    if command == "run" {
        return run::run_public_parent(&args[1..]);
    }

    if command == "pgo" {
        return match pgo::run(&args[1..]) {
            Ok(()) => 0,
            Err(message) => {
                print_error(&message);
                1
            }
        };
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

    if command == "lsp" {
        if args.len() != 1 {
            eprintln!("Usage: ckc lsp");
            return 2;
        }
        return lsp::run();
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
