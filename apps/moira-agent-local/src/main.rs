use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1);
    let Some(first) = arguments.next() else {
        eprintln!("usage: moira-agent-local [--check] CONFIG.json");
        return ExitCode::from(2);
    };
    let (check, path) = if first == "--check" {
        let Some(path) = arguments.next() else {
            eprintln!("usage: moira-agent-local [--check] CONFIG.json");
            return ExitCode::from(2);
        };
        (true, path)
    } else {
        (false, first)
    };
    if arguments.next().is_some() {
        eprintln!("usage: moira-agent-local [--check] CONFIG.json");
        return ExitCode::from(2);
    }
    let config = match moira_agent_local::load(path).and_then(moira_agent_local::validate) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if check {
        println!("moira agent local configuration valid");
        return ExitCode::SUCCESS;
    }
    match moira_agent_local::execute(config) {
        Ok(summary) => {
            println!("{}", summary.json());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
