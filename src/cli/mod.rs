mod args;
mod commands;
mod output;
mod toolchain;

use args::usage;
use commands::dispatch;

pub(crate) fn run(args: Vec<String>) -> i32 {
    let Some(command) = args.first().map(String::as_str) else {
        eprint!("{}", usage());
        return 2;
    };

    if command == "--help" || command == "-h" {
        print!("{}", usage());
        return 0;
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
