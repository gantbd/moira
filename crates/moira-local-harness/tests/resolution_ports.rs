use moira_core::deliberation::{
    AuthorityDecisionIdV1, AuthorityRefV1, AuthoritySubjectIdV1, CandidateIdV1, CandidateV1,
    ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1, DeliberationIdV1,
    DeliberationPlanV1, EnsembleMemberIdV1, EnsembleMemberV1, GroundingRefV1, MemberReportV1,
    MethodRefV1, OrthogonalityEvidenceV1, PersonaLensV1, ReportIdV1, ResolutionV1, SurfaceDepthV1,
    registry::{DeliberationRegistryV1, RegistryCommandV1, RegistryLimitsV1, RegistryRevisionV1},
};
use moira_host::{DeliberationApplyErrorV1, DeliberationStore};
use moira_local_harness::resolution::{
    BooleanValidatorV1, ChoiceAuthorityV1, ChoiceDecisionV1, ChoiceRequestV1,
    ChoiceResolutionErrorV1, ProofRequestV1, ProofResolutionErrorV1, ProofWitnessV1,
    resolve_authorized_choice, resolve_proven_bool,
};
use std::convert::Infallible;

struct Store(DeliberationRegistryV1);

impl Store {
    fn ready() -> Self {
        let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
        registry
            .apply(RegistryRevisionV1(0), RegistryCommandV1::Admit(plan()))
            .unwrap();
        registry
            .apply(
                RegistryRevisionV1(1),
                RegistryCommandV1::SubmitReport {
                    deliberation: DeliberationIdV1([1; 16]),
                    report: report(),
                },
            )
            .unwrap();
        Self(registry)
    }

    fn collecting() -> Self {
        let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
        registry
            .apply(RegistryRevisionV1(0), RegistryCommandV1::Admit(plan()))
            .unwrap();
        Self(registry)
    }
}

impl DeliberationStore for Store {
    type Error = Infallible;

    fn revision(&self) -> RegistryRevisionV1 {
        self.0.revision()
    }

    fn needs_reopen(&self) -> bool {
        false
    }

    fn deliberation(
        &self,
        id: DeliberationIdV1,
    ) -> Option<moira_core::deliberation::DeliberationCheckpointV1> {
        self.0.deliberation(id)
    }

    fn apply(
        &mut self,
        expected_revision: RegistryRevisionV1,
        command: RegistryCommandV1,
    ) -> Result<
        moira_core::deliberation::registry::RegistryReceiptV1,
        DeliberationApplyErrorV1<Self::Error>,
    > {
        self.0
            .apply(expected_revision, command)
            .map_err(DeliberationApplyErrorV1::Domain)
    }
}

fn plan() -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([1; 16]),
        question_digest: [2; 32],
        method: MethodRefV1 {
            machine_digest: [3; 32],
            state_digest: [4; 32],
        },
        claims: vec![ClaimKindV1::Formal],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([5; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![
            CandidateV1 {
                id: CandidateIdV1([6; 16]),
                scores: vec![5],
            },
            CandidateV1 {
                id: CandidateIdV1([7; 16]),
                scores: vec![3],
            },
        ],
        admission: moira_core::deliberation::EnsembleAdmissionV1::affirmed(
            [14; 32], [15; 32], [16; 32], [17; 32], [18; 16],
        ),
        members: vec![EnsembleMemberV1 {
            id: EnsembleMemberIdV1([8; 16]),
            stakeholder: moira_core::deliberation::StakeholderRefV1::technical_sme(
                [8; 16], [9; 32],
            ),
            lens: PersonaLensV1::Theorist,
            concern_root_digest: [9; 32],
            grounding: GroundingRefV1 {
                authority_digest: [10; 32],
                surface_digest: [11; 32],
                depth: SurfaceDepthV1::Activated,
            },
        }],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [12; 32],
            result_digest: [13; 32],
            all_orthogonal: true,
        },
        fuel: 2,
    }
}

fn report() -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([14; 16]),
        member: EnsembleMemberIdV1([8; 16]),
        output_digest: [15; 32],
        evidence_digests: vec![[2; 32], [16; 32]],
        dissent_digests: vec![[17; 32]],
    }
}

struct Validator {
    calls: usize,
    mismatch: bool,
}

impl BooleanValidatorV1 for Validator {
    type Error = Infallible;

    fn validate(&mut self, request: ProofRequestV1) -> Result<ProofWitnessV1, Self::Error> {
        self.calls += 1;
        let mut subject = request.subject;
        if self.mismatch {
            subject.question_digest = [99; 32];
        }
        Ok(ProofWitnessV1 {
            subject,
            validator: [18; 16],
            value: true,
            evidence_digest: [19; 32],
        })
    }
}

struct Authority {
    selected: CandidateIdV1,
    calls: usize,
}

impl ChoiceAuthorityV1 for Authority {
    type Error = Infallible;

    fn choose(&mut self, request: ChoiceRequestV1) -> Result<ChoiceDecisionV1, Self::Error> {
        self.calls += 1;
        Ok(ChoiceDecisionV1 {
            subject: request.subject,
            authority: AuthorityRefV1 {
                decision: AuthorityDecisionIdV1([20; 16]),
                subject: AuthoritySubjectIdV1([21; 16]),
                epoch: 22,
            },
            selected: self.selected,
        })
    }
}

#[test]
fn exact_validator_witness_resolves_the_bound_question() {
    let mut store = Store::ready();
    let mut validator = Validator {
        calls: 0,
        mismatch: false,
    };
    let receipt =
        resolve_proven_bool(&mut store, DeliberationIdV1([1; 16]), &mut validator).unwrap();
    assert_eq!(validator.calls, 1);
    assert_eq!(receipt.revision, RegistryRevisionV1(3));
    assert_eq!(
        receipt.deliberation.resolution,
        Some(ResolutionV1::ProvenBool {
            value: true,
            validator: [18; 16],
            evidence_digest: [19; 32]
        })
    );
}

#[test]
fn mismatched_witness_and_incomplete_reports_refuse_without_mutation() {
    let mut ready = Store::ready();
    let mut mismatched = Validator {
        calls: 0,
        mismatch: true,
    };
    assert!(matches!(
        resolve_proven_bool(&mut ready, DeliberationIdV1([1; 16]), &mut mismatched),
        Err(ProofResolutionErrorV1::InvalidWitness)
    ));
    assert_eq!(ready.revision(), RegistryRevisionV1(2));

    let mut collecting = Store::collecting();
    let mut untouched = Validator {
        calls: 0,
        mismatch: false,
    };
    assert!(matches!(
        resolve_proven_bool(&mut collecting, DeliberationIdV1([1; 16]), &mut untouched),
        Err(ProofResolutionErrorV1::ReportsIncomplete)
    ));
    assert_eq!(untouched.calls, 0);
    assert_eq!(collecting.revision(), RegistryRevisionV1(1));
}

#[test]
fn authority_cannot_select_a_dominated_candidate() {
    let mut store = Store::ready();
    let mut invalid = Authority {
        selected: CandidateIdV1([7; 16]),
        calls: 0,
    };
    assert!(matches!(
        resolve_authorized_choice(&mut store, DeliberationIdV1([1; 16]), &mut invalid),
        Err(ChoiceResolutionErrorV1::InvalidDecision)
    ));
    assert_eq!(invalid.calls, 1);
    assert_eq!(store.revision(), RegistryRevisionV1(2));

    let mut valid = Authority {
        selected: CandidateIdV1([6; 16]),
        calls: 0,
    };
    let receipt =
        resolve_authorized_choice(&mut store, DeliberationIdV1([1; 16]), &mut valid).unwrap();
    assert_eq!(
        receipt.deliberation.resolution,
        Some(ResolutionV1::AuthorizedChoice {
            frontier: vec![CandidateIdV1([6; 16])],
            selected: CandidateIdV1([6; 16]),
            authority: AuthorityRefV1 {
                decision: AuthorityDecisionIdV1([20; 16]),
                subject: AuthoritySubjectIdV1([21; 16]),
                epoch: 22,
            }
        })
    );
}

#[cfg(unix)]
#[test]
#[ignore = "requires the sibling Rune OCaml workspace to be built"]
fn real_rune_t6_prover_resolves_only_its_exact_bound_question() {
    use moira_rune_validator::{RuneTheoremValidatorConfigV1, RuneTheoremValidatorV1};
    use std::{path::PathBuf, time::Duration};

    let program = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../rune/src/ocaml/_build/default/t6_prover.exe");
    let mut validator = RuneTheoremValidatorV1::new(RuneTheoremValidatorConfigV1 {
        program,
        arguments: Vec::new(),
        expected_question_digest: [2; 32],
        validator: *b"RUNE-T6-PT4-V001",
        max_output_bytes: 65_536,
        timeout: Duration::from_secs(5),
    })
    .unwrap();
    let mut store = Store::ready();
    let receipt =
        resolve_proven_bool(&mut store, DeliberationIdV1([1; 16]), &mut validator).unwrap();
    assert!(matches!(
        receipt.deliberation.resolution,
        Some(ResolutionV1::ProvenBool {
            value: true,
            validator: id,
            ..
        }) if id == *b"RUNE-T6-PT4-V001"
    ));
}
