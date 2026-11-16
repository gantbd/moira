use moira_core::saga::{
    EffectOutcomeV1, OperatorActionIdV1, OperatorCauseV1, OperatorDispositionV1, SagaCommandV1,
    SagaEngineV1, SagaLimitsV1, SagaTransitionV1,
    persistence::{
        SagaCodecErrorV1, SagaCodecLimitsV1, decode_saga_checkpoint, encode_saga_checkpoint,
    },
};

fn decode_fixture(source: &str) -> Vec<u8> {
    let source = source.trim();
    assert_eq!(source.len() % 2, 0);
    source
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(text, 16).unwrap()
        })
        .collect()
}

fn limits() -> SagaCodecLimitsV1 {
    SagaCodecLimitsV1 {
        max_bytes: 8_192,
        max_sagas: 4,
        max_operator_actions: 8,
    }
}

#[test]
fn canonical_saga_checkpoint_roundtrips_and_retains_exact_replay() {
    let mut engine = SagaEngineV1::try_new(SagaLimitsV1 {
        max_sagas: 4,
        max_retries_per_saga: 2,
        max_inspection_page: 2,
        max_operator_actions: 8,
    })
    .unwrap();
    let command = SagaCommandV1::Admit {
        source: [7; 16],
        now: 10,
        deadline: 20,
        retry_budget: 2,
    };
    let admitted = engine.apply(0, command).unwrap();
    let SagaTransitionV1::Admitted { saga, effect, .. } = admitted else {
        panic!("unexpected transition")
    };
    let accepted = SagaCommandV1::AcceptEffect {
        saga,
        effect,
        outcome: EffectOutcomeV1::Failed,
        observed_at: 11,
    };
    engine.apply(1, accepted).unwrap();
    let retry = SagaCommandV1::Retry {
        saga,
        failed_effect: effect,
        now: 12,
    };
    engine.apply(2, retry).unwrap();

    let encoded = encode_saga_checkpoint(&engine.checkpoint(), limits()).unwrap();
    assert_eq!(
        encoded.as_bytes(),
        decode_fixture(include_str!("fixtures/saga-checkpoint-v1-live.hex"))
    );
    let decoded = decode_saga_checkpoint(encoded.as_bytes(), limits()).unwrap();
    assert_eq!(
        encode_saga_checkpoint(&decoded, limits())
            .unwrap()
            .as_bytes(),
        encoded.as_bytes()
    );
    let mut recovered = SagaEngineV1::recover(decoded).unwrap();
    assert_eq!(recovered.apply(3, command).unwrap(), admitted);
    assert_eq!(
        recovered.apply(3, accepted).unwrap(),
        engine.apply(3, accepted).unwrap()
    );
    assert_eq!(
        recovered.apply(3, retry).unwrap(),
        engine.apply(3, retry).unwrap()
    );
}

#[test]
fn saga_codec_refuses_truncation_trailing_bytes_and_host_bounds() {
    let engine = SagaEngineV1::try_new(SagaLimitsV1 {
        max_sagas: 2,
        max_retries_per_saga: 1,
        max_inspection_page: 1,
        max_operator_actions: 8,
    })
    .unwrap();
    let encoded = encode_saga_checkpoint(&engine.checkpoint(), limits()).unwrap();
    for end in 0..encoded.as_bytes().len() {
        assert!(decode_saga_checkpoint(&encoded.as_bytes()[..end], limits()).is_err());
    }
    let mut trailing = encoded.as_bytes().to_vec();
    trailing.push(0);
    assert_eq!(
        decode_saga_checkpoint(&trailing, limits()),
        Err(SagaCodecErrorV1::Trailing)
    );
    assert!(
        encode_saga_checkpoint(
            &engine.checkpoint(),
            SagaCodecLimitsV1 {
                max_bytes: 8,
                max_sagas: 4,
                max_operator_actions: 8,
            }
        )
        .is_err()
    );
    assert!(
        decode_saga_checkpoint(
            encoded.as_bytes(),
            SagaCodecLimitsV1 {
                max_bytes: 8_192,
                max_sagas: 1,
                max_operator_actions: 8,
            }
        )
        .is_err()
    );
}

#[test]
fn saga_codec_retains_operator_causation_and_lost_reply_receipt() {
    let mut engine = SagaEngineV1::try_new(SagaLimitsV1 {
        max_sagas: 2,
        max_retries_per_saga: 1,
        max_inspection_page: 1,
        max_operator_actions: 8,
    })
    .unwrap();
    let SagaTransitionV1::Admitted { saga, effect, .. } = engine
        .apply(
            0,
            SagaCommandV1::Admit {
                source: [3; 16],
                now: 1,
                deadline: 10,
                retry_budget: 1,
            },
        )
        .unwrap()
    else {
        panic!("admission expected")
    };
    engine
        .apply(
            1,
            SagaCommandV1::AcceptEffect {
                saga,
                effect,
                outcome: EffectOutcomeV1::Uncertain,
                observed_at: 2,
            },
        )
        .unwrap();
    let command = SagaCommandV1::Reconcile {
        cause: OperatorCauseV1 {
            action: OperatorActionIdV1([4; 16]),
            subject: [5; 16],
            authority_decision: 6,
            authority_epoch: 7,
        },
        saga,
        disposition: OperatorDispositionV1::KeepUncertain,
    };
    let transition = engine.apply(2, command).unwrap();
    let encoded = encode_saga_checkpoint(&engine.checkpoint(), limits()).unwrap();
    let mut recovered =
        SagaEngineV1::recover(decode_saga_checkpoint(encoded.as_bytes(), limits()).unwrap())
            .unwrap();
    assert_eq!(recovered.apply(2, command).unwrap(), transition);
    assert_eq!(recovered.revision(), 3);
}
