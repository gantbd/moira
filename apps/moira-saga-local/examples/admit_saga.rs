use moira_client::SagaMutationClient;
use moira_client::unix::{SagaClientLimitsV1, SagaUnixPort};
use moira_protocol::{SagaMutationV1, SagaOutcomeV1, SagaReplyV1, SagaSourceV1};
use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1);
    let Some(socket) = arguments.next() else {
        eprintln!("usage: admit_saga SOCKET");
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        eprintln!("usage: admit_saga SOCKET");
        return ExitCode::from(2);
    }
    let port = match SagaUnixPort::connect(socket, SagaClientLimitsV1::default()) {
        Ok(port) => port,
        Err(error) => {
            eprintln!("Moira connection refused: {error:?}");
            return ExitCode::FAILURE;
        }
    };
    let mut client = SagaMutationClient::new(port);
    match client.apply(
        0,
        SagaMutationV1::Admit {
            source: SagaSourceV1([0x51; 16]),
            now: 10,
            deadline: 20,
            retry_budget: 1,
        },
    ) {
        Ok(SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            storage_revision,
            replayed: false,
            ..
        })) => {
            println!("{storage_revision}");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("Moira admission refused: {other:?}");
            ExitCode::FAILURE
        }
    }
}
