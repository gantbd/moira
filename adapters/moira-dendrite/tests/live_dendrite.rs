use moira_core::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
    DeliberationIdV1, EnsembleAdmissionV1, EnsembleMemberIdV1, GroundingSourceProvenanceV1,
    PersonaLensV1, StakeholderRefV1, SurfaceDepthV1,
    grounding::{self, MemberIntentV1},
};
use moira_dendrite::{
    ClientLimitsV1, DendriteClient, GroundedMemberRequestV1, PlanGroundingRequestV1,
    concern_root_digest,
};

#[test]
fn live_fold_surfaces_and_orthogonality_form_one_verified_ensemble() {
    let Ok(port) = std::env::var("MOIRA_DENDRITE_LIVE_PORT") else {
        return;
    };
    let port = port.parse::<u16>().unwrap();
    let bearer = std::env::var("MOIRA_DENDRITE_BEARER").ok();
    let mut client =
        DendriteClient::connect_loopback(port, bearer, ClientLimitsV1::default()).unwrap();
    let method = client
        .resolve_method("rune/fsm/behavior-driven-development", "discovery")
        .unwrap();
    assert_ne!(method.machine_digest, [0; 32]);
    assert_ne!(method.state_digest, [0; 32]);
    assert_eq!(
        client
            .state_context_tags("rune/fsm/behavior-driven-development", "discovery")
            .unwrap(),
        vec!["architecture-flow-review", "behavior-example-design"]
    );
    let refs = [
        (EnsembleMemberIdV1([1; 16]), "rune:architecture".to_owned()),
        (EnsembleMemberIdV1([2; 16]), "rune:network".to_owned()),
        (EnsembleMemberIdV1([3; 16]), "rune:identity".to_owned()),
        (EnsembleMemberIdV1([4; 16]), "rune:data".to_owned()),
    ];
    let fold = client
        .query_fold(
            "rune/sme/triggers",
            &["architecture-flow-review".to_owned()],
            &refs,
        )
        .unwrap();
    let surfaces = refs
        .iter()
        .map(|(member, target)| {
            let authority = fold
                .iter()
                .find(|value| value.member == *member)
                .unwrap()
                .authority_digest;
            client
                .query_surface(
                    "rune/sme/triggers",
                    target,
                    *member,
                    authority,
                    SurfaceDepthV1::Ambient,
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    let orthogonality = client
        .query_orthogonality("axon/sme/axon-concerns", &refs)
        .unwrap();
    let lattice_digest = client.content_identity("axon/sme/axon-concerns").unwrap();
    let fold_digest = client.content_identity("rune/sme/triggers").unwrap();
    let assembled = grounding::assemble(
        &refs
            .iter()
            .enumerate()
            .map(|(index, (id, _))| MemberIntentV1 {
                id: *id,
                stakeholder: StakeholderRefV1::technical_sme(id.0, [index as u8 + 1; 32]),
                lens: if index == 0 {
                    PersonaLensV1::Theorist
                } else {
                    PersonaLensV1::RigorReviewer
                },
                concern_root_digest: concern_root_digest(lattice_digest, &refs[index].1).unwrap(),
            })
            .collect::<Vec<_>>(),
        &fold,
        &surfaces,
        orthogonality,
    )
    .unwrap();
    assert_eq!(assembled.members.len(), 4);
    assert!(assembled.orthogonality.all_orthogonal);

    let plan = client
        .ground_plan(PlanGroundingRequestV1 {
            id: DeliberationIdV1([20; 16]),
            question_digest: [23; 32],
            method_load: "rune/fsm/behavior-driven-development".to_owned(),
            method_state: "discovery".to_owned(),
            fold_load: "rune/sme/triggers".to_owned(),
            fold_tags: vec!["architecture-flow-review".to_owned()],
            orthogonality_load: "axon/sme/axon-concerns".to_owned(),
            claims: vec![ClaimKindV1::Empirical],
            criteria: vec![CriterionV1 {
                id: CriterionIdV1([21; 16]),
                direction: CriterionDirectionV1::Maximize,
            }],
            candidates: vec![CandidateV1 {
                id: CandidateIdV1([22; 16]),
                scores: vec![1],
            }],
            admission: EnsembleAdmissionV1::affirmed(
                [24; 32], [25; 32], [26; 32], [27; 32], [28; 16],
            ),
            members: refs
                .iter()
                .enumerate()
                .map(|(index, (id, target))| GroundedMemberRequestV1 {
                    id: *id,
                    stakeholder: StakeholderRefV1::technical_sme(id.0, [index as u8 + 1; 32]),
                    lens: if index == 0 {
                        PersonaLensV1::Theorist
                    } else {
                        PersonaLensV1::RigorReviewer
                    },
                    fold_target: target.clone(),
                    orthogonality_role: target.clone(),
                    depth: SurfaceDepthV1::Ambient,
                })
                .collect(),
            fuel: 3,
        })
        .unwrap();
    assert_eq!(plan.method, method);
    assert_eq!(
        plan.grounding_provenance,
        Some(GroundingSourceProvenanceV1 {
            fold_content_digest: fold_digest,
            orthogonality_content_digest: lattice_digest,
        })
    );
    assert_eq!(plan.members, assembled.members);
    assert_eq!(plan.orthogonality, assembled.orthogonality);
}
