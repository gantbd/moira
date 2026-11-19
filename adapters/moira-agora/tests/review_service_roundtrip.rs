use agora_client::v2::{PortFailureV2, ServicePortV2};
use agora_engine::{
    AdmissionPolicy, CommandV2 as EngineCommand, Engine, Participant, ParticipantKind, RoomId,
};
use agora_protocol::v2::{ClientIdV2, ReplyV2, RequestV2};
use agora_service::v2::{ServiceLimitsV2, ServiceV2};
use moira_agora::{
    PublicationLimitsV1, PublicationReplyV1, ResponsePublisher,
    review::{ReviewPublicationRefV1, ReviewPublicationRouteV1, project_review_publication},
};
use moira_core::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
    DeliberationIdV1, DeliberationPlanV1, EnsembleAdmissionV1, EnsembleMemberIdV1,
    EnsembleMemberV1, FindingDispositionIdV1, FindingIdV1, GroundingRefV1, MemberReportV1,
    MethodRefV1, OrthogonalityEvidenceV1, PersonaLensV1, RebuttalIdV1, ReportIdV1,
    StakeholderRefV1, SurfaceDepthV1,
    registry::{DeliberationRegistryV1, RegistryCommandV1, RegistryLimitsV1},
    review::{FindingDispositionKindV1, FindingDispositionV1, RebuttalV1, ReviewFindingV1},
};
use moira_protocol::EffectIdV1;
use std::convert::Infallible;

struct RealAgoraPort {
    service: ServiceV2,
}

impl ServicePortV2 for RealAgoraPort {
    type Error = Infallible;

    fn execute(
        &mut self,
        client: ClientIdV2,
        request: RequestV2,
    ) -> Result<ReplyV2, PortFailureV2<Self::Error>> {
        Ok(self.service.execute(client, request))
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
        claims: vec![ClaimKindV1::Empirical],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([5; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: CandidateIdV1([6; 16]),
            scores: vec![1],
        }],
        admission: EnsembleAdmissionV1::affirmed([7; 32], [8; 32], [9; 32], [10; 32], [11; 16]),
        members: vec![
            EnsembleMemberV1 {
                id: EnsembleMemberIdV1([12; 16]),
                stakeholder: StakeholderRefV1::technical_sme([12; 16], [13; 32]),
                lens: PersonaLensV1::Researcher,
                concern_root_digest: [14; 32],
                grounding: GroundingRefV1 {
                    authority_digest: [15; 32],
                    surface_digest: [16; 32],
                    depth: SurfaceDepthV1::Activated,
                },
            },
            EnsembleMemberV1 {
                id: EnsembleMemberIdV1([17; 16]),
                stakeholder: StakeholderRefV1::technical_sme([17; 16], [18; 32]),
                lens: PersonaLensV1::RigorReviewer,
                concern_root_digest: [19; 32],
                grounding: GroundingRefV1 {
                    authority_digest: [20; 32],
                    surface_digest: [21; 32],
                    depth: SurfaceDepthV1::Reference,
                },
            },
        ],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [22; 32],
            result_digest: [23; 32],
            all_orthogonal: true,
        },
        fuel: 4,
    }
}

fn report(id: u8, member: u8) -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([id; 16]),
        member: EnsembleMemberIdV1([member; 16]),
        output_digest: [id; 32],
        evidence_digests: vec![[id.wrapping_add(1); 32]],
        dissent_digests: vec![[id.wrapping_add(2); 32]],
    }
}

fn retained_review() -> moira_core::deliberation::review::ReviewExchangeCheckpointV1 {
    let id = DeliberationIdV1([1; 16]);
    let mut registry = DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap();
    registry
        .apply(
            registry.revision(),
            RegistryCommandV1::Admit(Box::new(plan())),
        )
        .unwrap();
    for value in [report(24, 12), report(27, 17)] {
        registry
            .apply(
                registry.revision(),
                RegistryCommandV1::SubmitReport {
                    deliberation: id,
                    report: value,
                },
            )
            .unwrap();
    }
    registry
        .apply(
            registry.revision(),
            RegistryCommandV1::BeginReview { deliberation: id },
        )
        .unwrap();
    registry
        .apply(
            registry.revision(),
            RegistryCommandV1::SubmitFinding {
                deliberation: id,
                finding: ReviewFindingV1 {
                    id: FindingIdV1([30; 16]),
                    source_report: ReportIdV1([24; 16]),
                    owner: [31; 16],
                    finding_digest: [32; 32],
                    evidence_digests: vec![[33; 32]],
                    requires_human_gate: false,
                },
            },
        )
        .unwrap();
    registry
        .apply(
            registry.revision(),
            RegistryCommandV1::SubmitRebuttal {
                deliberation: id,
                rebuttal: RebuttalV1 {
                    id: RebuttalIdV1([34; 16]),
                    finding: FindingIdV1([30; 16]),
                    member: EnsembleMemberIdV1([17; 16]),
                    rebuttal_digest: [35; 32],
                    evidence_digests: vec![[36; 32]],
                },
            },
        )
        .unwrap();
    registry
        .apply(
            registry.revision(),
            RegistryCommandV1::DisposeFinding {
                deliberation: id,
                disposition: FindingDispositionV1 {
                    id: FindingDispositionIdV1([37; 16]),
                    finding: FindingIdV1([30; 16]),
                    owner: [31; 16],
                    kind: FindingDispositionKindV1::Mitigated,
                    rationale_digest: [38; 32],
                    evidence_digests: vec![[39; 32]],
                    human_gate_response: None,
                    decided_at: 40,
                },
            },
        )
        .unwrap();
    registry.review(id).unwrap()
}

fn agora() -> (ServiceV2, ClientIdV2) {
    let mut engine = Engine::new();
    for (id, kind) in [
        ("moira", ParticipantKind::Agent),
        ("owner", ParticipantKind::Human),
        ("advocate", ParticipantKind::Human),
    ] {
        engine
            .apply(EngineCommand::RegisterParticipant(Participant::new(
                id, id, kind,
            )))
            .unwrap();
    }
    engine
        .apply(EngineCommand::CreateRoom {
            room_id: "review-room".into(),
            name: "Review room".to_owned(),
            owner: "owner".into(),
            admission: AdmissionPolicy::Open,
        })
        .unwrap();
    for participant in ["moira", "advocate"] {
        engine
            .apply(EngineCommand::Join {
                room_id: "review-room".into(),
                participant: participant.into(),
            })
            .unwrap();
    }
    let mut service = ServiceV2::new(engine, ServiceLimitsV2::default());
    let client = service.connect("moira").unwrap();
    (service, client)
}

fn route(effect: u8, expected_agora_revision: u64) -> ReviewPublicationRouteV1 {
    ReviewPublicationRouteV1 {
        effect: EffectIdV1([effect; 16]),
        source: [41; 16],
        room_sequence: u64::from(effect),
        room_id: "review-room".to_owned(),
        expected_agora_revision,
        logical_time: u64::from(effect),
        addressed_to: vec!["owner".to_owned(), "advocate".to_owned()],
    }
}

#[test]
fn retained_review_facts_roundtrip_through_real_agora_service() {
    let review = retained_review();
    let (service, client) = agora();
    assert_eq!(service.engine().revision().0, 6);
    let mut publisher = ResponsePublisher::new(
        client,
        RealAgoraPort { service },
        PublicationLimitsV1::default(),
    )
    .unwrap();
    let publications = [
        ReviewPublicationRefV1::Finding(&review.findings[0]),
        ReviewPublicationRefV1::Rebuttal(&review.rebuttals[0]),
        ReviewPublicationRefV1::Disposition(&review.dispositions[0]),
    ];
    let mut bodies = Vec::new();
    let mut receipts = Vec::new();
    for (index, publication) in publications.into_iter().enumerate() {
        let effect = u8::try_from(index + 1).unwrap();
        let intent =
            project_review_publication(route(effect, 6 + index as u64), publication).unwrap();
        bodies.push(intent.body.clone());
        let outcome = publisher.publish(intent).unwrap();
        let PublicationReplyV1::Published(published) = outcome else {
            panic!("real Agora publication expected, got {outcome:?}");
        };
        receipts.push(published.receipt);
    }

    let (port, pending_intent, pending_request) = publisher.into_parts();
    assert!(pending_intent.is_none());
    assert!(pending_request.is_none());
    assert_eq!(port.service.engine().revision().0, 9);
    let room = RoomId::new("review-room");
    for sequence in 0..3 {
        let message = port.service.engine().message(&room, sequence).unwrap();
        assert_eq!(message.author().as_str(), "moira");
        assert_eq!(message.body(), bodies[sequence as usize]);
        assert_eq!(
            message
                .addressed_to()
                .iter()
                .map(|participant| participant.as_str())
                .collect::<Vec<_>>(),
            vec!["advocate", "owner"]
        );
        assert_eq!(
            receipts[sequence as usize].message.message_sequence,
            sequence
        );
        assert_eq!(receipts[sequence as usize].agora_revision, 7 + sequence);
    }
    assert!(bodies[0].contains("kind=finding"));
    assert!(bodies[1].contains("kind=rebuttal"));
    assert!(bodies[2].contains("kind=disposition"));
}
