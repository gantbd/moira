use moira_core::deliberation::{
    DeliberationCheckpointV1, DeliberationIdV1 as CoreDeliberationId,
    registry::{DeliberationRegistryV1, RegistryCommandV1, RegistryLimitsV1, RegistryRevisionV1},
    review::ReviewExchangeCheckpointV1,
};
use moira_host::{DeliberationApplyErrorV1, DeliberationStore};
use moira_protocol::deliberation::*;
use moira_service::{
    AccessV1,
    deliberation::{DeliberationService, HumanGateAccessV1},
};

fn correlation() -> EnsembleCorrelationEvidenceV1 {
    let mut value = EnsembleCorrelationEvidenceV1::undisclosed(2);
    value.dimensions[0] = CorrelationDimensionEvidenceV1 {
        dimension: CorrelationDimensionV1::ExecutionPrincipal,
        observed_members: 2,
        distinct_values: 1,
    };
    value.first_pass = FirstPassExposureV1 {
        committed_before_cross_review: 2,
        exposed_before_commit: 0,
        unknown: 0,
    };
    value
}

struct Store {
    registry: DeliberationRegistryV1,
    fenced: bool,
}

impl Store {
    fn new() -> Self {
        Self {
            registry: DeliberationRegistryV1::try_new(RegistryLimitsV1::default()).unwrap(),
            fenced: false,
        }
    }
}

impl DeliberationStore for Store {
    type Error = ();

    fn revision(&self) -> RegistryRevisionV1 {
        self.registry.revision()
    }

    fn needs_reopen(&self) -> bool {
        self.fenced
    }

    fn deliberation(&self, id: CoreDeliberationId) -> Option<DeliberationCheckpointV1> {
        self.registry.deliberation(id)
    }

    fn review(&self, id: CoreDeliberationId) -> Option<ReviewExchangeCheckpointV1> {
        self.registry.review(id)
    }

    fn apply(
        &mut self,
        expected_revision: RegistryRevisionV1,
        command: RegistryCommandV1,
    ) -> Result<
        moira_core::deliberation::registry::RegistryReceiptV1,
        DeliberationApplyErrorV1<Self::Error>,
    > {
        self.registry
            .apply(expected_revision, command)
            .map_err(DeliberationApplyErrorV1::Domain)
    }
}

fn plan(seed: u8) -> DeliberationPlanV1 {
    DeliberationPlanV1 {
        id: DeliberationIdV1([seed; 16]),
        question_digest: [9; 32],
        method: MethodRefV1 {
            machine_digest: [10; 32],
            state_digest: [11; 32],
        },
        grounding_provenance: Some(GroundingSourceProvenanceV1 {
            fold_content_digest: [26; 32],
            orthogonality_content_digest: [27; 32],
        }),
        claims: vec![ClaimKindV1::Historical, ClaimKindV1::Empirical],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([12; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: CandidateIdV1([13; 16]),
            scores: vec![7],
        }],
        admission: EnsembleAdmissionV1::affirmed([21; 32], [22; 32], [23; 32], [24; 32], [25; 16]),
        members: vec![EnsembleMemberV1 {
            id: EnsembleMemberIdV1([14; 16]),
            stakeholder: StakeholderRefV1::technical_sme([14; 16], [15; 32]),
            lens: PersonaLensV1::Historian,
            concern_root_digest: [15; 32],
            grounding: GroundingRefV1 {
                authority_digest: [16; 32],
                surface_digest: [17; 32],
                depth: SurfaceDepthV1::Activated,
            },
        }],
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [18; 32],
            result_digest: [19; 32],
            all_orthogonal: true,
        },
        fuel: 2,
    }
}

fn report() -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([20; 16]),
        member: EnsembleMemberIdV1([14; 16]),
        output_digest: [21; 32],
        evidence_digests: vec![[22; 32]],
        dissent_digests: vec![[23; 32]],
    }
}

fn review_plan() -> DeliberationPlanV1 {
    let mut value = plan(1);
    value.fuel = 3;
    value.members.push(EnsembleMemberV1 {
        id: EnsembleMemberIdV1([15; 16]),
        stakeholder: StakeholderRefV1::technical_sme([15; 16], [16; 32]),
        lens: PersonaLensV1::RigorReviewer,
        concern_root_digest: [16; 32],
        grounding: GroundingRefV1 {
            authority_digest: [17; 32],
            surface_digest: [18; 32],
            depth: SurfaceDepthV1::Reference,
        },
    });
    value
}

fn second_report() -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([24; 16]),
        member: EnsembleMemberIdV1([15; 16]),
        output_digest: [25; 32],
        evidence_digests: vec![[26; 32]],
        dissent_digests: vec![[27; 32]],
    }
}

#[test]
fn service_preserves_grounding_dissent_fuel_resolution_and_replay() {
    let mut service = DeliberationService::new(Store::new());
    let admitted = service
        .execute(CommandV1::Apply {
            expected_revision: 0,
            mutation: MutationV1::Admit(Box::new(plan(1))),
        })
        .unwrap();
    assert!(matches!(
        admitted,
        ReplyV1::Accepted(OutcomeV1::Applied {
            revision: 1,
            replayed: false,
            ref deliberation,
        }) if deliberation.phase == DeliberationPhaseV1::Collecting
            && deliberation.plan.method.machine_digest == [10; 32]
            && deliberation.plan.members[0].grounding.surface_digest == [17; 32]
            && deliberation.remaining_fuel == 2
    ));

    assert!(matches!(
        service
            .execute(CommandV1::Apply {
                expected_revision: 1,
                mutation: MutationV1::Admit(Box::new(plan(1))),
            })
            .unwrap(),
        ReplyV1::Accepted(OutcomeV1::Applied {
            revision: 1,
            replayed: true,
            ..
        })
    ));

    service
        .execute(CommandV1::Apply {
            expected_revision: 1,
            mutation: MutationV1::SubmitReport {
                deliberation: DeliberationIdV1([1; 16]),
                report: report(),
            },
        })
        .unwrap();
    service
        .execute(CommandV1::Apply {
            expected_revision: 2,
            mutation: MutationV1::Resolve {
                deliberation: DeliberationIdV1([1; 16]),
                resolution: ResolutionV1::Maybe(MaybeValueV1::Both),
            },
        })
        .unwrap();

    let ReplyV1::Accepted(OutcomeV1::Deliberation(Some(view))) = service
        .execute(CommandV1::Deliberation {
            deliberation: DeliberationIdV1([1; 16]),
        })
        .unwrap()
    else {
        panic!("retained deliberation expected");
    };
    assert_eq!(view.phase, DeliberationPhaseV1::Resolved);
    assert_eq!(view.reports[0].dissent_digests, vec![[23; 32]]);
    assert_eq!(view.remaining_fuel, 0);
    assert_eq!(
        view.resolution,
        Some(ResolutionV1::Maybe(MaybeValueV1::Both))
    );
}

#[test]
fn access_and_optimistic_revision_refuse_before_mutation() {
    let mut service = DeliberationService::new(Store::new());
    assert_eq!(
        service
            .execute_authorized(
                AccessV1 {
                    read: true,
                    mutate: false
                },
                CommandV1::Apply {
                    expected_revision: 0,
                    mutation: MutationV1::Admit(Box::new(plan(1))),
                },
            )
            .unwrap(),
        ReplyV1::Refused(RefusalV1::NotAuthorized)
    );
    assert!(matches!(
        service.execute(CommandV1::Status).unwrap(),
        ReplyV1::Accepted(OutcomeV1::Status(StatusV1 { revision: 0, .. }))
    ));

    service
        .execute(CommandV1::Apply {
            expected_revision: 0,
            mutation: MutationV1::Admit(Box::new(plan(1))),
        })
        .unwrap();
    assert_eq!(
        service
            .execute(CommandV1::Apply {
                expected_revision: 0,
                mutation: MutationV1::Admit(Box::new(plan(2))),
            })
            .unwrap(),
        ReplyV1::Refused(RefusalV1::RevisionConflict {
            expected: 0,
            actual: 1
        })
    );
    assert_eq!(
        service
            .execute_authorized(
                AccessV1 {
                    read: false,
                    mutate: true
                },
                CommandV1::Deliberation {
                    deliberation: DeliberationIdV1([1; 16])
                },
            )
            .unwrap(),
        ReplyV1::Refused(RefusalV1::NotAuthorized)
    );
}

#[test]
fn domain_refusals_remain_typed_at_the_public_boundary() {
    let mut service = DeliberationService::new(Store::new());
    let mut invalid = plan(1);
    invalid.criteria.clear();
    assert_eq!(
        service
            .execute(CommandV1::Apply {
                expected_revision: 0,
                mutation: MutationV1::Admit(Box::new(invalid)),
            })
            .unwrap(),
        ReplyV1::Refused(RefusalV1::Deliberation(DeliberationRefusalV1::Frontier(
            FrontierRefusalV1::NoCriteria
        )))
    );
}

#[test]
fn public_review_commands_retain_challenge_disposition_and_read_projection() {
    let mut service = DeliberationService::new(Store::new());
    service
        .execute(CommandV1::Apply {
            expected_revision: 0,
            mutation: MutationV1::Admit(Box::new(review_plan())),
        })
        .unwrap();
    for (expected_revision, value) in [(1, report()), (2, second_report())] {
        service
            .execute(CommandV1::Apply {
                expected_revision,
                mutation: MutationV1::SubmitReport {
                    deliberation: DeliberationIdV1([1; 16]),
                    report: value,
                },
            })
            .unwrap();
    }

    assert!(matches!(
        service
            .execute(CommandV1::Apply {
                expected_revision: 3,
                mutation: MutationV1::BeginReview {
                    deliberation: DeliberationIdV1([1; 16]),
                    correlation: correlation(),
                },
            })
            .unwrap(),
        ReplyV1::Accepted(OutcomeV1::ReviewApplied {
            revision: 4,
            replayed: false,
            ref review,
            ..
        }) if review.phase == ReviewPhaseV1::FirstPassComplete
            && review.reports.len() == 2
            && review.independence == IndependenceAssessmentV1::Correlated
            && review.correlation.first_pass.committed_before_cross_review == 2
    ));

    service
        .execute(CommandV1::Apply {
            expected_revision: 4,
            mutation: MutationV1::SubmitFinding {
                deliberation: DeliberationIdV1([1; 16]),
                finding: ReviewFindingV1 {
                    id: FindingIdV1([30; 16]),
                    source_report: ReportIdV1([20; 16]),
                    owner: [31; 16],
                    finding_digest: [32; 32],
                    evidence_digests: vec![[33; 32]],
                    requires_human_gate: false,
                },
            },
        })
        .unwrap();
    assert!(matches!(
        service
            .execute(CommandV1::Apply {
                expected_revision: 5,
                mutation: MutationV1::SubmitRebuttal {
                    deliberation: DeliberationIdV1([1; 16]),
                    rebuttal: RebuttalV1 {
                        id: RebuttalIdV1([34; 16]),
                        finding: FindingIdV1([30; 16]),
                        member: EnsembleMemberIdV1([15; 16]),
                        rebuttal_digest: [35; 32],
                        evidence_digests: vec![[36; 32]],
                    },
                },
            })
            .unwrap(),
        ReplyV1::Accepted(OutcomeV1::ReviewApplied { ref review, .. })
            if review.phase == ReviewPhaseV1::Challenged
    ));
    service
        .execute(CommandV1::Apply {
            expected_revision: 6,
            mutation: MutationV1::DisposeFinding {
                deliberation: DeliberationIdV1([1; 16]),
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
        })
        .unwrap();

    let ReplyV1::Accepted(OutcomeV1::Review(Some(review))) = service
        .execute(CommandV1::Review {
            deliberation: DeliberationIdV1([1; 16]),
        })
        .unwrap()
    else {
        panic!("retained review expected");
    };
    assert_eq!(review.phase, ReviewPhaseV1::Disposed);
    assert_eq!(review.findings.len(), 1);
    assert_eq!(review.rebuttals.len(), 1);
    assert_eq!(review.dispositions.len(), 1);
}

#[test]
fn named_human_response_updates_review_replays_and_authorizes_gated_disposition() {
    let mut service = DeliberationService::new(Store::new());
    service
        .execute(CommandV1::Apply {
            expected_revision: 0,
            mutation: MutationV1::Admit(Box::new(review_plan())),
        })
        .unwrap();
    for (expected_revision, value) in [(1, report()), (2, second_report())] {
        service
            .execute(CommandV1::Apply {
                expected_revision,
                mutation: MutationV1::SubmitReport {
                    deliberation: DeliberationIdV1([1; 16]),
                    report: value,
                },
            })
            .unwrap();
    }
    service
        .execute(CommandV1::Apply {
            expected_revision: 3,
            mutation: MutationV1::Resolve {
                deliberation: DeliberationIdV1([1; 16]),
                resolution: ResolutionV1::AuthorizedChoice {
                    frontier: vec![CandidateIdV1([13; 16])],
                    selected: CandidateIdV1([13; 16]),
                    authority: AuthorityRefV1 {
                        decision: AuthorityDecisionIdV1([71; 16]),
                        subject: AuthoritySubjectIdV1([72; 16]),
                        epoch: 73,
                    },
                },
            },
        })
        .unwrap();
    service
        .execute(CommandV1::Apply {
            expected_revision: 4,
            mutation: MutationV1::RequestHumanGate {
                deliberation: DeliberationIdV1([1; 16]),
                request: HumanGateRequestV1 {
                    id: HumanGateRequestIdV1([41; 16]),
                    reviewer: [42; 16],
                    finding_digest: [43; 32],
                    decision_digest: [44; 32],
                    material_revision_digest: [45; 32],
                    policy_digest: [46; 32],
                    decision_packet: HumanGateMaterialRefV1 {
                        owner_route: [47; 16],
                        artifact_route: [48; 16],
                        expected_revision: 49,
                        expected_sha256: [50; 32],
                    },
                    requested_at: 51,
                },
            },
        })
        .unwrap();
    service
        .execute(CommandV1::Apply {
            expected_revision: 5,
            mutation: MutationV1::BeginReview {
                deliberation: DeliberationIdV1([1; 16]),
                correlation: correlation(),
            },
        })
        .unwrap();
    service
        .execute(CommandV1::Apply {
            expected_revision: 6,
            mutation: MutationV1::SubmitFinding {
                deliberation: DeliberationIdV1([1; 16]),
                finding: ReviewFindingV1 {
                    id: FindingIdV1([52; 16]),
                    source_report: ReportIdV1([20; 16]),
                    owner: [53; 16],
                    finding_digest: [43; 32],
                    evidence_digests: vec![[54; 32]],
                    requires_human_gate: true,
                },
            },
        })
        .unwrap();
    service
        .execute(CommandV1::Apply {
            expected_revision: 7,
            mutation: MutationV1::SubmitRebuttal {
                deliberation: DeliberationIdV1([1; 16]),
                rebuttal: RebuttalV1 {
                    id: RebuttalIdV1([55; 16]),
                    finding: FindingIdV1([52; 16]),
                    member: EnsembleMemberIdV1([15; 16]),
                    rebuttal_digest: [56; 32],
                    evidence_digests: vec![[57; 32]],
                },
            },
        })
        .unwrap();

    let operation = HumanGateOperationV1 {
        deliberation: DeliberationIdV1([1; 16]),
        expected_revision: 8,
        response: HumanGateResponseSubmissionV1 {
            id: HumanGateResponseIdV1([58; 16]),
            request: HumanGateRequestIdV1([41; 16]),
            disposition: HumanGateDispositionV1::Approved,
            rationale_digest: [59; 32],
            rationale: HumanGateMaterialRefV1 {
                owner_route: [60; 16],
                artifact_route: [61; 16],
                expected_revision: 62,
                expected_sha256: [63; 32],
            },
            responded_at: 64,
        },
    };
    let verified = HumanGateAccessV1 {
        decision: [65; 16],
        subject: [42; 16],
        epoch: 66,
    };
    for replayed in [false, true] {
        let reply = service
            .execute_human_gate_authorized(
                AccessV1 {
                    read: true,
                    mutate: true,
                },
                verified,
                operation,
            )
            .unwrap();
        assert!(matches!(
            reply,
            ReplyV1::Accepted(OutcomeV1::ReviewApplied {
                revision: 9,
                replayed: actual,
                ref review,
                ..
            }) if actual == replayed
                && review.admitted_human_gate_response.as_ref().is_some_and(|response|
                    response.id == HumanGateResponseIdV1([58; 16])
                        && response.disposition == HumanGateDispositionV1::Approved)
        ));
    }

    assert!(matches!(
        service
            .execute(CommandV1::Apply {
                expected_revision: 9,
                mutation: MutationV1::DisposeFinding {
                    deliberation: DeliberationIdV1([1; 16]),
                    disposition: FindingDispositionV1 {
                        id: FindingDispositionIdV1([67; 16]),
                        finding: FindingIdV1([52; 16]),
                        owner: [53; 16],
                        kind: FindingDispositionKindV1::RiskAccepted,
                        rationale_digest: [68; 32],
                        evidence_digests: vec![[69; 32]],
                        human_gate_response: Some(HumanGateResponseIdV1([58; 16])),
                        decided_at: 70,
                    },
                },
            })
            .unwrap(),
        ReplyV1::Accepted(OutcomeV1::ReviewApplied {
            revision: 10,
            ref review,
            ..
        }) if review.phase == ReviewPhaseV1::Disposed
    ));
}

#[test]
fn human_gate_identity_is_injected_by_authority_and_exact_lost_reply_replays() {
    let mut service = DeliberationService::new(Store::new());
    service
        .execute(CommandV1::Apply {
            expected_revision: 0,
            mutation: MutationV1::Admit(Box::new(plan(1))),
        })
        .unwrap();
    service
        .execute(CommandV1::Apply {
            expected_revision: 1,
            mutation: MutationV1::SubmitReport {
                deliberation: DeliberationIdV1([1; 16]),
                report: report(),
            },
        })
        .unwrap();
    service
        .execute(CommandV1::Apply {
            expected_revision: 2,
            mutation: MutationV1::Resolve {
                deliberation: DeliberationIdV1([1; 16]),
                resolution: ResolutionV1::Maybe(MaybeValueV1::Both),
            },
        })
        .unwrap();
    service
        .execute(CommandV1::Apply {
            expected_revision: 3,
            mutation: MutationV1::RequestHumanGate {
                deliberation: DeliberationIdV1([1; 16]),
                request: HumanGateRequestV1 {
                    id: HumanGateRequestIdV1([30; 16]),
                    reviewer: [31; 16],
                    finding_digest: [32; 32],
                    decision_digest: [33; 32],
                    material_revision_digest: [34; 32],
                    policy_digest: [35; 32],
                    decision_packet: HumanGateMaterialRefV1 {
                        owner_route: [42; 16],
                        artifact_route: [43; 16],
                        expected_revision: 7,
                        expected_sha256: [44; 32],
                    },
                    requested_at: 36,
                },
            },
        })
        .unwrap();
    let operation = HumanGateOperationV1 {
        deliberation: DeliberationIdV1([1; 16]),
        expected_revision: 4,
        response: HumanGateResponseSubmissionV1 {
            id: HumanGateResponseIdV1([37; 16]),
            request: HumanGateRequestIdV1([30; 16]),
            disposition: HumanGateDispositionV1::Approved,
            rationale_digest: [38; 32],
            rationale: HumanGateMaterialRefV1 {
                owner_route: [45; 16],
                artifact_route: [46; 16],
                expected_revision: 8,
                expected_sha256: [47; 32],
            },
            responded_at: 39,
        },
    };

    assert_eq!(
        service
            .execute(CommandV1::RespondHumanGate { operation })
            .unwrap(),
        ReplyV1::Refused(RefusalV1::NotAuthorized)
    );
    assert_eq!(
        service
            .execute_human_gate_authorized(
                AccessV1 {
                    read: true,
                    mutate: true,
                },
                HumanGateAccessV1 {
                    decision: [40; 16],
                    subject: [41; 16],
                    epoch: 42,
                },
                operation,
            )
            .unwrap(),
        ReplyV1::Refused(RefusalV1::Deliberation(
            DeliberationRefusalV1::HumanGateReviewerMismatch
        ))
    );

    let verified = HumanGateAccessV1 {
        decision: [40; 16],
        subject: [31; 16],
        epoch: 42,
    };
    let accepted = service
        .execute_human_gate_authorized(
            AccessV1 {
                read: true,
                mutate: true,
            },
            verified,
            operation,
        )
        .unwrap();
    assert!(matches!(
        accepted,
        ReplyV1::Accepted(OutcomeV1::Applied {
            revision: 5,
            replayed: false,
            ref deliberation,
        }) if deliberation.human_gate_response.as_ref().is_some_and(|response|
            response.authority.subject == [31; 16]
                && response.authority.decision == [40; 16]
                && response.disposition == HumanGateDispositionV1::Approved)
    ));

    assert!(matches!(
        service
            .execute_human_gate_authorized(
                AccessV1 {
                    read: true,
                    mutate: true,
                },
                verified,
                operation,
            )
            .unwrap(),
        ReplyV1::Accepted(OutcomeV1::Applied {
            revision: 5,
            replayed: true,
            ..
        })
    ));
}
