use moira_core::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
    DeliberationCheckpointV1, DeliberationIdV1, DeliberationPlanV1, EnsembleAdmissionV1,
    EnsembleMemberIdV1, EnsembleMemberV1, GroundingRefV1, MemberReportV1, MethodRefV1,
    OrthogonalityEvidenceV1, PersonaLensV1, ReportIdV1, StakeholderRefV1, SurfaceDepthV1,
    registry::{
        DeliberationRegistryV1, RegistryCommandV1, RegistryLimitsV1, RegistryReceiptV1,
        RegistryRevisionV1,
    },
};
use moira_daimon::ensemble::{MemberResponseLimitsV1, MemberResponseRefusalV1};
use moira_host::{
    AgentOutputCommitV1, AgentOutputRecordV1, AgentOutputStoreV1, DeliberationApplyErrorV1,
    DeliberationStore,
};
use moira_local_harness::agent_report::{
    AgentReportAdmissionErrorV1, admit_terminal_output, admit_terminal_report,
};
use moira_protocol::{WorkflowIdV1, agent_participation as p, deliberation as d};
use sha2::{Digest, Sha256};
use std::convert::Infallible;

struct Store(DeliberationRegistryV1);

struct Outputs {
    bytes: Option<Vec<u8>>,
}

impl AgentOutputStoreV1 for Outputs {
    type Error = Infallible;

    fn append(
        &mut self,
        participation: [u8; 16],
        turn: [u8; 16],
        _after: Option<u64>,
        through: u64,
        _page_digest: [u8; 32],
        assistant_bytes: &[u8],
    ) -> Result<AgentOutputCommitV1, Self::Error> {
        Ok(AgentOutputCommitV1 {
            record: AgentOutputRecordV1 {
                participation,
                turn,
                last_cursor: through,
                output_digest: Sha256::digest(assistant_bytes).into(),
                output_bytes: assistant_bytes.len(),
            },
            replayed: false,
        })
    }

    fn get(
        &mut self,
        participation: [u8; 16],
        turn: [u8; 16],
    ) -> Result<Option<(AgentOutputRecordV1, Vec<u8>)>, Self::Error> {
        Ok(self.bytes.clone().map(|bytes| {
            (
                AgentOutputRecordV1 {
                    participation,
                    turn,
                    last_cursor: 2,
                    output_digest: Sha256::digest(&bytes).into(),
                    output_bytes: bytes.len(),
                },
                bytes,
            )
        }))
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

    fn deliberation(&self, id: DeliberationIdV1) -> Option<DeliberationCheckpointV1> {
        self.0.deliberation(id)
    }

    fn apply(
        &mut self,
        expected_revision: RegistryRevisionV1,
        command: RegistryCommandV1,
    ) -> Result<RegistryReceiptV1, DeliberationApplyErrorV1<Self::Error>> {
        self.0
            .apply(expected_revision, command)
            .map_err(DeliberationApplyErrorV1::Domain)
    }
}

#[test]
fn only_exact_grounded_terminal_evidence_becomes_a_report() {
    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan())),
        )
        .unwrap();
    let mut store = Store(registry);
    let mut outputs = outputs();
    let participation = participation();
    let mut missing_provider = report();
    missing_provider.evidence_digests.remove(0);
    assert_eq!(
        admit_terminal_report(&mut store, &mut outputs, &participation, missing_provider),
        Err(AgentReportAdmissionErrorV1::InvalidReport)
    );
    assert_eq!(store.revision(), RegistryRevisionV1(1));

    let accepted =
        admit_terminal_report(&mut store, &mut outputs, &participation, report()).unwrap();
    assert_eq!(accepted.revision, RegistryRevisionV1(2));
    assert!(!accepted.replayed);
    let replay = admit_terminal_report(&mut store, &mut outputs, &participation, report()).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.revision, RegistryRevisionV1(2));
}

#[test]
fn persona_output_and_completion_mismatch_refuse_before_store_mutation() {
    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan())),
        )
        .unwrap();
    let mut store = Store(registry);
    let mut outputs = outputs();
    let mut wrong_persona = participation();
    wrong_persona.binding.persona = d::PersonaLensV1::Historian;
    assert_eq!(
        admit_terminal_report(&mut store, &mut outputs, &wrong_persona, report()),
        Err(AgentReportAdmissionErrorV1::BindingMismatch)
    );
    let mut wrong_output = report();
    wrong_output.output_digest = [99; 32];
    assert_eq!(
        admit_terminal_report(&mut store, &mut outputs, &participation(), wrong_output),
        Err(AgentReportAdmissionErrorV1::InvalidReport)
    );
    let mut failed = participation();
    failed.terminal.as_mut().unwrap().disposition = p::DispositionV1::Failed;
    assert_eq!(
        admit_terminal_report(&mut store, &mut outputs, &failed, report()),
        Err(AgentReportAdmissionErrorV1::InvalidParticipation)
    );
    assert_eq!(store.revision(), RegistryRevisionV1(1));

    outputs.bytes = None;
    assert_eq!(
        admit_terminal_report(&mut store, &mut outputs, &participation(), report()),
        Err(AgentReportAdmissionErrorV1::MissingOutput)
    );
}

#[test]
fn retained_member_response_derives_one_stable_grounded_report() {
    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan())),
        )
        .unwrap();
    let mut store = Store(registry);
    let mut outputs = outputs();
    let participation = participation();

    let admitted = admit_terminal_output(
        &mut store,
        &mut outputs,
        &participation,
        MemberResponseLimitsV1::default(),
    )
    .unwrap();
    assert_eq!(admitted.receipt.revision, RegistryRevisionV1(2));
    assert!(!admitted.receipt.replayed);
    assert_ne!(admitted.report.id.0, [0; 16]);
    assert_eq!(admitted.report.member, EnsembleMemberIdV1([5; 16]));
    assert_eq!(admitted.report.output_digest, output_digest());
    assert_eq!(
        admitted.report.evidence_digests,
        vec![
            [1; 32], [9; 32], [10; 32], [11; 32], [12; 32], [21; 32], [0xaa; 32],
        ]
    );
    assert_eq!(admitted.report.dissent_digests, vec![[0xbb; 32]]);
    assert_eq!(admitted.response.claims.len(), 1);
    assert_eq!(admitted.response.claims[0].kind, ClaimKindV1::Empirical);
    assert_eq!(admitted.response.claims[0].statement, "retained claim");

    let replay = admit_terminal_output(
        &mut store,
        &mut outputs,
        &participation,
        MemberResponseLimitsV1::default(),
    )
    .unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.receipt.revision, RegistryRevisionV1(2));
    assert_eq!(replay.report.id, admitted.report.id);
    assert_eq!(replay.report, admitted.report);
}

#[test]
fn invalid_or_host_colliding_member_response_never_mutates_registry() {
    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    registry
        .apply(
            RegistryRevisionV1(0),
            RegistryCommandV1::Admit(Box::new(plan())),
        )
        .unwrap();
    let mut store = Store(registry);
    let participation = participation();
    let mut outputs = Outputs {
        bytes: Some(b"not json".to_vec()),
    };
    let mut invalid_participation = participation.clone();
    invalid_participation
        .terminal
        .as_mut()
        .unwrap()
        .output_digest = Sha256::digest(b"not json").into();
    assert_eq!(
        admit_terminal_output(
            &mut store,
            &mut outputs,
            &invalid_participation,
            MemberResponseLimitsV1::default(),
        ),
        Err(AgentReportAdmissionErrorV1::Response(
            MemberResponseRefusalV1::InvalidJson
        ))
    );
    assert_eq!(store.revision(), RegistryRevisionV1(1));

    let collision = br#"{"schema":"moira.member-response.v1","claims":[{"kind":"empirical","statement":"retained claim","evidence_digests":["0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b"],"dissent_digests":[]}]}"#
        .to_vec();
    outputs.bytes = Some(collision.clone());
    let mut collision_participation = participation;
    collision_participation
        .terminal
        .as_mut()
        .unwrap()
        .output_digest = Sha256::digest(&collision).into();
    assert_eq!(
        admit_terminal_output(
            &mut store,
            &mut outputs,
            &collision_participation,
            MemberResponseLimitsV1::default(),
        ),
        Err(AgentReportAdmissionErrorV1::InvalidReport)
    );
    assert_eq!(store.revision(), RegistryRevisionV1(1));
}

fn plan() -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([4; 16]),
        question_digest: [1; 32],
        method: MethodRefV1 {
            machine_digest: [9; 32],
            state_digest: [10; 32],
        },
        claims: vec![ClaimKindV1::Empirical],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([2; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: CandidateIdV1([3; 16]),
            scores: vec![1],
        }],
        admission: EnsembleAdmissionV1::affirmed([13; 32], [14; 32], [15; 32], [16; 32], [17; 16]),
        members: vec![EnsembleMemberV1 {
            id: EnsembleMemberIdV1([5; 16]),
            stakeholder: StakeholderRefV1::technical_sme([5; 16], [6; 32]),
            lens: PersonaLensV1::Researcher,
            concern_root_digest: [6; 32],
            grounding: GroundingRefV1 {
                authority_digest: [11; 32],
                surface_digest: [12; 32],
                depth: SurfaceDepthV1::Activated,
            },
        }],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [7; 32],
            result_digest: [8; 32],
            all_orthogonal: true,
        },
        fuel: 2,
    }
}

fn report() -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([30; 16]),
        member: EnsembleMemberIdV1([5; 16]),
        output_digest: output_digest(),
        evidence_digests: vec![[21; 32], [11; 32], [12; 32]],
        dissent_digests: vec![[22; 32]],
    }
}

fn participation() -> p::ParticipationViewV1 {
    p::ParticipationViewV1 {
        binding: p::BindingV1 {
            participation: p::ParticipationIdV1([2; 16]),
            workflow: WorkflowIdV1([3; 16]),
            deliberation: d::DeliberationIdV1([4; 16]),
            member: d::EnsembleMemberIdV1([5; 16]),
            session: p::AgentSessionIdV1([6; 16]),
            profile: p::ProfileIdV1([7; 16]),
            attempt: p::AttemptV1 {
                task_key: [8; 16],
                incarnation: 1,
                attempt: 1,
            },
            method: d::MethodRefV1 {
                machine_digest: [9; 32],
                state_digest: [10; 32],
            },
            grounding: d::GroundingRefV1 {
                authority_digest: [11; 32],
                surface_digest: [12; 32],
                depth: d::SurfaceDepthV1::Activated,
            },
            persona: d::PersonaLensV1::Researcher,
        },
        revision: 4,
        phase: p::PhaseV1::Terminal,
        session_revision: Some(3),
        last_event_cursor: Some(2),
        profile_document_digest: Some([13; 32]),
        capabilities: p::CapabilitiesV1::default(),
        active_turn: None,
        permission: None,
        terminal: Some(p::TerminalEvidenceV1 {
            turn: p::AgentTurnIdV1([14; 16]),
            disposition: p::DispositionV1::Completed,
            stop_reason: p::StopReasonV1::EndTurn,
            output_digest: output_digest(),
            provider_evidence_digest: [21; 32],
        }),
        pending: None,
    }
}

fn output_bytes() -> Vec<u8> {
    br#"{"schema":"moira.member-response.v1","claims":[{"kind":"empirical","statement":"retained claim","evidence_digests":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],"dissent_digests":["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"]}]}"#
        .to_vec()
}

fn output_digest() -> [u8; 32] {
    Sha256::digest(output_bytes()).into()
}

fn outputs() -> Outputs {
    Outputs {
        bytes: Some(output_bytes()),
    }
}
