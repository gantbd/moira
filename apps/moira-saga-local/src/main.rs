use std::{env, io, process::ExitCode};

const USAGE: &str = "usage: moira-saga-local [--check | --supervised] CONFIG.json";

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1);
    let Some(first) = arguments.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let (check, supervised, path) = if first == "--check" || first == "--supervised" {
        let Some(path) = arguments.next() else {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        };
        (first == "--check", first == "--supervised", path)
    } else {
        (false, false, first)
    };
    if arguments.next().is_some() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let config = match moira_saga_local::load(path) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if check {
        println!("Moira saga local configuration valid");
        return ExitCode::SUCCESS;
    }
    let result = if supervised {
        moira_saga_local::run_supervised(config, io::stdin())
    } else {
        moira_saga_local::run(config)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Moira saga owner stopped: {error}");
            ExitCode::FAILURE
        }
    }
}
