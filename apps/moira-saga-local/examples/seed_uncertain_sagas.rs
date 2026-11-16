use moira_client::SagaMutationClient;
use moira_client::unix::{SagaClientLimitsV1, SagaUnixPort};
use moira_protocol::{
    SagaEffectOutcomeV1, SagaMutationV1, SagaOutcomeV1, SagaReplyV1, SagaSourceV1, SagaTransitionV1,
};
use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1);
    let Some(socket) = arguments.next() else {
        eprintln!("usage: seed_uncertain_sagas SOCKET COUNT");
        return ExitCode::from(2);
    };
    let Some(count) = arguments
        .next()
        .and_then(|value| value.to_string_lossy().parse::<u8>().ok())
        .filter(|count| (1..=16).contains(count))
    else {
        eprintln!("COUNT must be in 1..=16");
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        eprintln!("usage: seed_uncertain_sagas SOCKET COUNT");
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
    let mut revision = 0;
    let mut sagas = Vec::with_capacity(usize::from(count));
    for index in 0..count {
        let reply = client.apply(
            revision,
            SagaMutationV1::Admit {
                source: SagaSourceV1([index.saturating_add(1); 16]),
                now: 10 + u64::from(index),
                deadline: 100,
                retry_budget: 1,
            },
        );
        let Ok(SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            storage_revision,
            transition: SagaTransitionV1::Admitted { saga, effect, .. },
            replayed: false,
        })) = reply
        else {
            eprintln!("Moira admission refused: {reply:?}");
            return ExitCode::FAILURE;
        };
        revision = storage_revision
            .checked_add(1)
            .expect("bounded seed revision");
        let reply = client.apply(
            revision,
            SagaMutationV1::AcceptEffect {
                saga,
                effect,
                outcome: SagaEffectOutcomeV1::Uncertain,
                observed_at: 20 + u64::from(index),
            },
        );
        let Ok(SagaReplyV1::Accepted(SagaOutcomeV1::Applied {
            storage_revision,
            replayed: false,
            ..
        })) = reply
        else {
            eprintln!("Moira uncertain effect refused: {reply:?}");
            return ExitCode::FAILURE;
        };
        revision = storage_revision
            .checked_add(1)
            .expect("bounded seed revision");
        sagas.push(saga.0);
    }
    println!(
        "{}",
        serde_json::json!({"storage_revision": revision, "sagas": sagas})
    );
    ExitCode::SUCCESS
}
