#![cfg(unix)]

use moira_host::{BooleanValidatorV1, ProofRequestV1, ResolutionSubjectV1};
use moira_rune_validator::{
    RuneTheoremValidatorConfigV1, RuneTheoremValidatorErrorV1, RuneTheoremValidatorV1,
};
use std::{ffi::OsString, path::PathBuf, time::Duration};

fn request(question: [u8; 32]) -> ProofRequestV1 {
    ProofRequestV1 {
        subject: ResolutionSubjectV1 {
            deliberation: moira_core::deliberation::DeliberationIdV1([1; 16]),
            question_digest: question,
            checkpoint_digest: [3; 32],
        },
    }
}

fn validator(script: &str, question: [u8; 32]) -> RuneTheoremValidatorV1 {
    RuneTheoremValidatorV1::new(RuneTheoremValidatorConfigV1 {
        program: PathBuf::from("/bin/sh"),
        arguments: vec![OsString::from("-c"), OsString::from(script)],
        expected_question_digest: question,
        validator: [4; 16],
        max_output_bytes: 4_096,
        timeout: Duration::from_secs(1),
    })
    .unwrap()
}

#[test]
fn exact_closed_world_grounded_output_earns_true_witness() {
    let output = "printf '%s\\n' '{\"verdict\":\"grounded\",\"lens_rune_id\":\"one\",\"tier\":\"pt-4\"}' '{\"aggregate\":true,\"grounded\":1,\"refused\":0,\"hold_quantum\":0,\"mu\":0,\"total\":1,\"tier\":\"pt-4\",\"anchor\":\"Mac Lane CWM §III.4\",\"promotion_path\":\"verified\"}'";
    let mut validator = validator(output, [2; 32]);
    let witness = validator.validate(request([2; 32])).unwrap();
    assert!(witness.value);
    assert_eq!(witness.subject.question_digest, [2; 32]);
    assert_eq!(witness.validator, [4; 16]);
}

#[test]
fn wrong_question_and_indeterminate_prover_output_never_promote() {
    let output = "printf '%s\\n' '{\"verdict\":\"hold_quantum\",\"lens_rune_id\":\"one\",\"tier\":\"pt-4\"}' '{\"aggregate\":true,\"grounded\":0,\"refused\":0,\"hold_quantum\":1,\"mu\":0,\"total\":1,\"tier\":\"pt-4\",\"anchor\":\"Mac Lane CWM §III.4\",\"promotion_path\":\"held\"}'";
    let mut validator = validator(output, [2; 32]);
    assert_eq!(
        validator.validate(request([9; 32])),
        Err(RuneTheoremValidatorErrorV1::WrongQuestion)
    );
    assert_eq!(
        validator.validate(request([2; 32])),
        Err(RuneTheoremValidatorErrorV1::InvalidOutput)
    );
}

#[test]
fn timeout_nonzero_exit_and_reconciled_count_mismatch_refuse() {
    let mut timed = validator("sleep 2", [2; 32]);
    assert_eq!(
        timed.validate(request([2; 32])),
        Err(RuneTheoremValidatorErrorV1::Timeout)
    );

    let mut failed = validator("exit 7", [2; 32]);
    assert_eq!(
        failed.validate(request([2; 32])),
        Err(RuneTheoremValidatorErrorV1::ProcessFailed)
    );

    let mismatched = "printf '%s\\n' '{\"verdict\":\"grounded\",\"lens_rune_id\":\"one\",\"tier\":\"pt-4\"}' '{\"aggregate\":true,\"grounded\":2,\"refused\":0,\"hold_quantum\":0,\"mu\":0,\"total\":2,\"tier\":\"pt-4\",\"anchor\":\"Mac Lane CWM §III.4\",\"promotion_path\":\"bad\"}'";
    let mut invalid = validator(mismatched, [2; 32]);
    assert_eq!(
        invalid.validate(request([2; 32])),
        Err(RuneTheoremValidatorErrorV1::InvalidOutput)
    );
}

#[test]
#[ignore = "requires the sibling Rune OCaml workspace to be built"]
fn live_rune_t6_prover_emits_a_bound_boolean_witness() {
    let program = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../rune/src/ocaml/_build/default/t6_prover.exe");
    let mut validator = RuneTheoremValidatorV1::new(RuneTheoremValidatorConfigV1 {
        program,
        arguments: Vec::new(),
        expected_question_digest: [21; 32],
        validator: *b"RUNE-T6-PT4-V001",
        max_output_bytes: 65_536,
        timeout: Duration::from_secs(5),
    })
    .unwrap();
    let witness = validator.validate(request([21; 32])).unwrap();
    assert!(witness.value);
    assert_eq!(witness.validator, *b"RUNE-T6-PT4-V001");
    assert_ne!(witness.evidence_digest, [0; 32]);
}
