use moira_core::deliberation::{
    AuthorityDecisionIdV1, AuthorityRefV1, AuthoritySubjectIdV1, CandidateIdV1, CandidateV1,
    ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1, DeliberationIdV1,
    DeliberationLimitsV1, DeliberationPlanV1, DeliberationV1, EnsembleMemberIdV1, EnsembleMemberV1,
    GroundingRefV1, MaybeValueV1, MemberReportV1, MethodRefV1, OrthogonalityEvidenceV1,
    PersonaLensV1, ReportIdV1, ResolutionV1, SurfaceDepthV1,
};
use moira_daimon::{
    ModelResultV1,
    ensemble::{MemberOutcomeV1, MemberResponseV1},
};
use moira_host::{ActionSelectionDecisionV1, ActionSelectionSubjectV1, ActionSelectorV1};
use moira_local_harness::action_selection::{
    ActionSelectionErrorV1, ActionSelectionEvidenceV1, select_authorized, select_single_member,
};
use moira_protocol::{DispositionV1, EffectIdV1, ResultReceiptV1, TaskRefV1};
use sha2::{Digest, Sha256};

fn plan() -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([1; 16]),
        question_digest: [2; 32],
        method: MethodRefV1 {
            machine_digest: [3; 32],
            state_digest: [4; 32],
        },
        claims: vec![ClaimKindV1::Empirical],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([22; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: CandidateIdV1([23; 16]),
            scores: vec![1],
        }],
        admission: moira_core::deliberation::EnsembleAdmissionV1::affirmed(
            [24; 32], [25; 32], [26; 32], [27; 32], [28; 16],
        ),
        members: vec![EnsembleMemberV1 {
            id: EnsembleMemberIdV1([5; 16]),
            stakeholder: moira_core::deliberation::StakeholderRefV1::technical_sme(
                [5; 16], [6; 32],
            ),
            lens: PersonaLensV1::Researcher,
            concern_root_digest: [6; 32],
            grounding: GroundingRefV1 {
                authority_digest: [7; 32],
                surface_digest: [8; 32],
                depth: SurfaceDepthV1::Ambient,
            },
        }],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [9; 32],
            result_digest: [10; 32],
            all_orthogonal: true,
        },
        fuel: 3,
    }
}

fn result(output: &[u8]) -> ModelResultV1 {
    ModelResultV1 {
        receipt: ResultReceiptV1 {
            effect: EffectIdV1([11; 16]),
            execution_id: [12; 16],
            task: TaskRefV1 {
                external_task_key: [13; 16],
                incarnation: 14,
            },
            attempt_number: 15,
            disposition: DispositionV1::Completed,
        },
        evidence_digest: [16; 32],
        invocation_evidence: None,
        input_tokens: 1,
        output_tokens: 1,
        output: output.to_vec(),
        proposals: Vec::new(),
    }
}

fn report(output: &[u8]) -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([17; 16]),
        member: EnsembleMemberIdV1([5; 16]),
        output_digest: Sha256::digest(output).into(),
        evidence_digests: Vec::new(),
        dissent_digests: Vec::new(),
    }
}

fn outcome(output: &[u8]) -> MemberOutcomeV1<()> {
    MemberOutcomeV1::Report {
        member: EnsembleMemberIdV1([5; 16]),
        report: report(output),
        response: MemberResponseV1 { claims: Vec::new() },
        result: Box::new(result(output)),
    }
}

fn checkpoint(resolved: bool) -> moira_core::deliberation::DeliberationCheckpointV1 {
    let mut deliberation =
        DeliberationV1::try_new(plan(), DeliberationLimitsV1::default()).unwrap();
    deliberation.submit_report(report(b"answer")).unwrap();
    if resolved {
        deliberation
            .resolve(ResolutionV1::AuthorizedChoice {
                frontier: vec![CandidateIdV1([23; 16])],
                selected: CandidateIdV1([23; 16]),
                authority: AuthorityRefV1 {
                    decision: AuthorityDecisionIdV1([24; 16]),
                    subject: AuthoritySubjectIdV1([25; 16]),
                    epoch: 26,
                },
            })
            .unwrap();
    }
    deliberation.checkpoint()
}

struct Selector {
    calls: usize,
}

impl ActionSelectorV1 for Selector {
    type Error = ();

    fn select(
        &mut self,
        subject: ActionSelectionSubjectV1,
    ) -> Result<ActionSelectionDecisionV1, Self::Error> {
        self.calls += 1;
        Ok(ActionSelectionDecisionV1 {
            subject,
            authority: AuthorityRefV1 {
                decision: AuthorityDecisionIdV1([18; 16]),
                subject: AuthoritySubjectIdV1([19; 16]),
                epoch: 20,
            },
        })
    }
}

#[test]
fn single_member_policy_binds_exact_retained_report_result_and_policy() {
    let checkpoint = checkpoint(false);
    let member_outcome = outcome(b"answer");
    let selected = select_single_member::<_, ()>(&checkpoint, &member_outcome, [21; 32]).unwrap();
    assert_eq!(selected.subject().report, ReportIdV1([17; 16]));
    assert_eq!(selected.subject().daimon_effect.into_bytes(), [11; 16]);
    assert_ne!(selected.selection_digest(), [0; 32]);
    assert_eq!(
        selected.evidence(),
        ActionSelectionEvidenceV1::SingleMemberPolicy {
            policy_digest: [21; 32]
        }
    );

    let changed = outcome(b"different");
    assert_eq!(
        select_single_member::<_, ()>(&checkpoint, &changed, [21; 32]),
        Err(ActionSelectionErrorV1::ReportNotRetained)
    );
}

#[test]
fn ensemble_action_requires_resolution_and_an_exact_authority_echo() {
    let outcome = outcome(b"answer");
    let mut selector = Selector { calls: 0 };
    assert_eq!(
        select_authorized(&checkpoint(false), &outcome, &mut selector),
        Err(ActionSelectionErrorV1::Unresolved)
    );
    assert_eq!(selector.calls, 0);

    let mut incompatible = checkpoint(false);
    incompatible.resolution = Some(ResolutionV1::Maybe(MaybeValueV1::TrueOnly));
    assert_eq!(
        select_authorized(&incompatible, &outcome, &mut selector),
        Err(ActionSelectionErrorV1::IncompatibleResolution)
    );
    assert_eq!(selector.calls, 0);

    let selected = select_authorized(&checkpoint(true), &outcome, &mut selector).unwrap();
    assert_eq!(selector.calls, 1);
    assert!(matches!(
        selected.evidence(),
        ActionSelectionEvidenceV1::Authorized { .. }
    ));
    assert_ne!(selected.selection_digest(), [0; 32]);
}
