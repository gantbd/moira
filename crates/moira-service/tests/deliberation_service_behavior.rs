use moira_core::deliberation::{
    DeliberationCheckpointV1, DeliberationIdV1 as CoreDeliberationId,
    registry::{DeliberationRegistryV1, RegistryCommandV1, RegistryLimitsV1, RegistryRevisionV1},
};
use moira_host::{DeliberationApplyErrorV1, DeliberationStore};
use moira_protocol::deliberation::*;
use moira_service::{
    AccessV1,
    deliberation::{DeliberationService, HumanGateAccessV1},
};

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
