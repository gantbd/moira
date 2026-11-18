use moira_protocol::deliberation as d;
use moira_wire::{
    CodecErrorV1, LimitsV1, decode_deliberation_reply, decode_deliberation_request,
    decode_saga_request, encode_deliberation_reply, encode_deliberation_request,
};

fn plan() -> d::DeliberationPlanV1 {
    d::DeliberationPlanV1 {
        id: d::DeliberationIdV1([1; 16]),
        question_digest: [2; 32],
        method: d::MethodRefV1 {
            machine_digest: [3; 32],
            state_digest: [4; 32],
        },
        claims: vec![
            d::ClaimKindV1::Formal,
            d::ClaimKindV1::Empirical,
            d::ClaimKindV1::Normative,
            d::ClaimKindV1::Historical,
            d::ClaimKindV1::Creative,
        ],
        criteria: vec![
            d::CriterionV1 {
                id: d::CriterionIdV1([5; 16]),
                direction: d::CriterionDirectionV1::Maximize,
            },
            d::CriterionV1 {
                id: d::CriterionIdV1([6; 16]),
                direction: d::CriterionDirectionV1::Minimize,
            },
        ],
        candidates: vec![
            d::CandidateV1 {
                id: d::CandidateIdV1([7; 16]),
                scores: vec![8, -9],
            },
            d::CandidateV1 {
                id: d::CandidateIdV1([10; 16]),
                scores: vec![11, -12],
            },
        ],
        admission: d::EnsembleAdmissionV1::affirmed(
            [24; 32], [25; 32], [26; 32], [27; 32], [28; 16],
        ),
        members: vec![
            member(13, d::PersonaLensV1::Historian, d::SurfaceDepthV1::Ambient),
            member(
                14,
                d::PersonaLensV1::IdeaStormer,
                d::SurfaceDepthV1::Activated,
            ),
            member(
                15,
                d::PersonaLensV1::ConvergenceFacilitator,
                d::SurfaceDepthV1::Reference,
            ),
        ],
        orthogonality: d::OrthogonalityEvidenceV1 {
            query_digest: [16; 32],
            result_digest: [17; 32],
            all_orthogonal: true,
        },
        fuel: 18,
    }
}

fn member(seed: u8, lens: d::PersonaLensV1, depth: d::SurfaceDepthV1) -> d::EnsembleMemberV1 {
    d::EnsembleMemberV1 {
        id: d::EnsembleMemberIdV1([seed; 16]),
        stakeholder: d::StakeholderRefV1::technical_sme([seed; 16], [seed; 32]),
        lens,
        concern_root_digest: [seed.wrapping_add(1); 32],
        grounding: d::GroundingRefV1 {
            authority_digest: [seed.wrapping_add(2); 32],
            surface_digest: [seed.wrapping_add(3); 32],
            depth,
        },
    }
}

fn report() -> d::MemberReportV1 {
    d::MemberReportV1 {
        id: d::ReportIdV1([19; 16]),
        member: d::EnsembleMemberIdV1([13; 16]),
        output_digest: [20; 32],
        evidence_digests: vec![[21; 32], [22; 32]],
        dissent_digests: vec![[23; 32]],
    }
}

fn choice() -> d::ResolutionV1 {
    d::ResolutionV1::AuthorizedChoice {
        frontier: vec![d::CandidateIdV1([7; 16]), d::CandidateIdV1([10; 16])],
        selected: d::CandidateIdV1([10; 16]),
        authority: d::AuthorityRefV1 {
            decision: d::AuthorityDecisionIdV1([24; 16]),
            subject: d::AuthoritySubjectIdV1([25; 16]),
            epoch: 26,
        },
    }
}

fn view(resolution: Option<d::ResolutionV1>) -> d::DeliberationViewV1 {
    d::DeliberationViewV1 {
        version: 1,
        plan: plan(),
        limits: d::DeliberationLimitsV1 {
            max_claims: 5,
            max_criteria: 2,
            max_candidates: 2,
            max_members: 3,
            max_evidence_per_report: 2,
            max_dissent_per_report: 1,
            max_fuel: 50,
        },
        reports: vec![report()],
        resolution,
        human_gate_request: Some(d::HumanGateRequestV1 {
            id: d::HumanGateRequestIdV1([33; 16]),
            reviewer: [34; 16],
            finding_digest: [35; 32],
            decision_digest: [36; 32],
            material_revision_digest: [37; 32],
            policy_digest: [38; 32],
            requested_at: 39,
        }),
        human_gate_response: Some(d::HumanGateResponseV1 {
            id: d::HumanGateResponseIdV1([40; 16]),
            request: d::HumanGateRequestIdV1([33; 16]),
            disposition: d::HumanGateDispositionV1::ChangesRequested,
            rationale_digest: [41; 32],
            responded_at: 42,
            authority: d::HumanGateAuthorityRefV1 {
                decision: [43; 16],
                subject: [34; 16],
                epoch: 44,
            },
        }),
        remaining_fuel: 27,
        phase: d::DeliberationPhaseV1::Resolved,
    }
}

#[test]
fn every_command_family_roundtrips_exactly() {
    let limits = LimitsV1::default();
    let values = [
        d::CommandV1::Apply {
            expected_revision: 28,
            mutation: d::MutationV1::Admit(Box::new(plan())),
        },
        d::CommandV1::Apply {
            expected_revision: 31,
            mutation: d::MutationV1::RequestHumanGate {
                deliberation: d::DeliberationIdV1([1; 16]),
                request: d::HumanGateRequestV1 {
                    id: d::HumanGateRequestIdV1([33; 16]),
                    reviewer: [34; 16],
                    finding_digest: [35; 32],
                    decision_digest: [36; 32],
                    material_revision_digest: [37; 32],
                    policy_digest: [38; 32],
                    requested_at: 39,
                },
            },
        },
        d::CommandV1::RespondHumanGate {
            operation: d::HumanGateOperationV1 {
                deliberation: d::DeliberationIdV1([1; 16]),
                expected_revision: 32,
                response: d::HumanGateResponseSubmissionV1 {
                    id: d::HumanGateResponseIdV1([40; 16]),
                    request: d::HumanGateRequestIdV1([33; 16]),
                    disposition: d::HumanGateDispositionV1::Approved,
                    rationale_digest: [41; 32],
                    responded_at: 42,
                },
            },
        },
        d::CommandV1::Apply {
            expected_revision: 29,
            mutation: d::MutationV1::SubmitReport {
                deliberation: d::DeliberationIdV1([1; 16]),
                report: report(),
            },
        },
        d::CommandV1::Apply {
            expected_revision: 30,
            mutation: d::MutationV1::Resolve {
                deliberation: d::DeliberationIdV1([1; 16]),
                resolution: choice(),
            },
        },
        d::CommandV1::Status,
        d::CommandV1::Deliberation {
            deliberation: d::DeliberationIdV1([1; 16]),
        },
    ];
    for value in values {
        let bytes = encode_deliberation_request(&value, limits).unwrap();
        assert_eq!(decode_deliberation_request(&bytes, limits).unwrap(), value);
        assert_eq!(
            decode_saga_request(&bytes, limits),
            Err(CodecErrorV1::Unsupported)
        );
    }
}

#[test]
fn prior_deliberation_profile_frames_refuse_instead_of_reinterpreting_stakeholder_evidence() {
    assert_eq!(moira_wire::DELIBERATION_PROFILE_VERSION, 3);
    let limits = LimitsV1::default();
    let mut request = encode_deliberation_request(&d::CommandV1::Status, limits).unwrap();
    request[8] = 10;
    assert_eq!(
        decode_deliberation_request(&request, limits),
        Err(CodecErrorV1::Unsupported)
    );

    let mut reply = encode_deliberation_reply(
        &d::ReplyV1::Accepted(d::OutcomeV1::Status(d::StatusV1 {
            revision: 0,
            needs_reopen: false,
        })),
        limits,
    )
    .unwrap();
    reply[8] = 11;
    assert_eq!(
        decode_deliberation_reply(&reply, limits),
        Err(CodecErrorV1::Unsupported)
    );
}

#[test]
fn every_resolution_family_roundtrips_in_a_complete_view() {
    let limits = LimitsV1::default();
    let values = [
        d::ResolutionV1::ProvenBool {
            value: true,
            validator: [31; 16],
            evidence_digest: [32; 32],
        },
        choice(),
        d::ResolutionV1::Maybe(d::MaybeValueV1::Both),
        d::ResolutionV1::Mu(d::MuReasonV1::ContradictoryEvidence),
    ];
    for resolution in values {
        let value = d::ReplyV1::Accepted(d::OutcomeV1::Applied {
            revision: 33,
            deliberation: Box::new(view(Some(resolution))),
            replayed: true,
        });
        let bytes = encode_deliberation_reply(&value, limits).unwrap();
        assert_eq!(decode_deliberation_reply(&bytes, limits).unwrap(), value);
    }
}

#[test]
fn status_optional_view_and_all_refusals_roundtrip() {
    let limits = LimitsV1::default();
    let outcomes = [
        d::ReplyV1::Accepted(d::OutcomeV1::Status(d::StatusV1 {
            revision: 34,
            needs_reopen: true,
        })),
        d::ReplyV1::Accepted(d::OutcomeV1::Deliberation(None)),
        d::ReplyV1::Accepted(d::OutcomeV1::Deliberation(Some(Box::new(view(None))))),
    ];
    for value in outcomes {
        let bytes = encode_deliberation_reply(&value, limits).unwrap();
        assert_eq!(decode_deliberation_reply(&bytes, limits).unwrap(), value);
    }

    let mut refusals = vec![
        d::RefusalV1::NotAuthorized,
        d::RefusalV1::InvalidLimits,
        d::RefusalV1::UnsupportedVersion,
        d::RefusalV1::RevisionConflict {
            expected: 35,
            actual: 36,
        },
        d::RefusalV1::RevisionExhausted,
        d::RefusalV1::Capacity,
        d::RefusalV1::UnknownDeliberation,
        d::RefusalV1::InputConflict,
        d::RefusalV1::DuplicateDeliberation,
        d::RefusalV1::CheckpointMismatch,
    ];
    let domain = [
        d::DeliberationRefusalV1::UnsupportedVersion,
        d::DeliberationRefusalV1::InvalidLimits,
        d::DeliberationRefusalV1::EmptyClaims,
        d::DeliberationRefusalV1::ClaimLimit,
        d::DeliberationRefusalV1::CriterionLimit,
        d::DeliberationRefusalV1::CandidateLimit,
        d::DeliberationRefusalV1::EmptyEnsemble,
        d::DeliberationRefusalV1::MemberLimit,
        d::DeliberationRefusalV1::DuplicateMember,
        d::DeliberationRefusalV1::NonOrthogonalEnsemble,
        d::DeliberationRefusalV1::FuelInvalid,
        d::DeliberationRefusalV1::UnknownMember,
        d::DeliberationRefusalV1::ReportLimit,
        d::DeliberationRefusalV1::ReportConflict,
        d::DeliberationRefusalV1::AlreadyResolved,
        d::DeliberationRefusalV1::ReportsIncomplete,
        d::DeliberationRefusalV1::ResolutionConflict,
        d::DeliberationRefusalV1::FrontierMismatch,
        d::DeliberationRefusalV1::SelectionOutsideFrontier,
        d::DeliberationRefusalV1::FuelRemaining,
        d::DeliberationRefusalV1::CheckpointMismatch,
    ];
    refusals.extend(domain.into_iter().map(d::RefusalV1::Deliberation));
    for value in [
        d::FrontierRefusalV1::NoCriteria,
        d::FrontierRefusalV1::NoCandidates,
        d::FrontierRefusalV1::DuplicateCriterion,
        d::FrontierRefusalV1::DuplicateCandidate,
        d::FrontierRefusalV1::ScoreDimensionMismatch,
    ] {
        refusals.push(d::RefusalV1::Deliberation(
            d::DeliberationRefusalV1::Frontier(value),
        ));
    }
    for refusal in refusals {
        let value = d::ReplyV1::Refused(refusal);
        let bytes = encode_deliberation_reply(&value, limits).unwrap();
        assert_eq!(decode_deliberation_reply(&bytes, limits).unwrap(), value);
    }
}

#[test]
fn truncation_trailing_bytes_invalid_tags_and_limits_fail_closed() {
    let limits = LimitsV1::default();
    let value = d::CommandV1::Apply {
        expected_revision: 37,
        mutation: d::MutationV1::Admit(Box::new(plan())),
    };
    let bytes = encode_deliberation_request(&value, limits).unwrap();
    for end in 0..bytes.len() {
        assert!(decode_deliberation_request(&bytes[..end], limits).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        decode_deliberation_request(&trailing, limits),
        Err(CodecErrorV1::Malformed)
    );
    assert_eq!(
        encode_deliberation_request(
            &value,
            LimitsV1 {
                max_frame_bytes: bytes.len() - 1,
            },
        ),
        Err(CodecErrorV1::Limit)
    );

    let mut invalid = encode_deliberation_request(&d::CommandV1::Status, limits).unwrap();
    *invalid.last_mut().unwrap() = u8::MAX;
    assert_eq!(
        decode_deliberation_request(&invalid, limits),
        Err(CodecErrorV1::Unsupported)
    );
}

#[test]
fn declared_collection_count_cannot_force_unbounded_allocation() {
    let limits = LimitsV1::default();
    let value = d::CommandV1::Apply {
        expected_revision: 37,
        mutation: d::MutationV1::Admit(Box::new(plan())),
    };
    let mut bytes = encode_deliberation_request(&value, limits).unwrap();
    // magic + frame/command/mutation tags + revision + plan id/question/method;
    // overwrite the first collection count (claims).
    bytes[131..135].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(
        decode_deliberation_request(&bytes, limits),
        Err(CodecErrorV1::Limit)
    );
}
