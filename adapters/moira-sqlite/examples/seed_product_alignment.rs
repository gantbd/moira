#![forbid(unsafe_code)]

use moira_core::deliberation::{
    DeliberationLimitsV1,
    registry::{RegistryLimitsV1, persistence::CodecLimitsV1},
};
use moira_protocol::deliberation::{
    CandidateIdV1, CandidateV1, ClaimKindV1, CommandV1, CriterionDirectionV1, CriterionIdV1,
    CriterionV1, DeliberationIdV1, DeliberationPlanV1, EnsembleAdmissionV1, EnsembleMemberIdV1,
    EnsembleMemberV1, GroundingRefV1, GroundingSourceProvenanceV1, HumanGateMaterialRefV1,
    HumanGateRequestIdV1, HumanGateRequestV1, MaybeValueV1, MemberReportV1, MethodRefV1,
    MutationV1, OrthogonalityEvidenceV1, OutcomeV1, PersonaLensV1, ReplyV1, ReportIdV1,
    ResolutionV1, StakeholderRefV1, StakeholderRepresentationV1, SurfaceDepthV1,
};
use moira_service::deliberation::DeliberationService;
use moira_sqlite::deliberation::{DeliberationSqliteStore, DeliberationStoreLimitsV1};
use serde::Deserialize;
use std::{error::Error, fs, path::PathBuf};

const DELIBERATION: [u8; 16] = [1; 16];
const EXTERNAL_ADVOCATE: [u8; 16] = [6; 16];
const PRODUCT_MANAGER: [u8; 16] = [17; 16];
const FRONTIER_SUPERVISOR: [u8; 16] = [18; 16];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TellusFactsV1 {
    version: u8,
    method_sha256: String,
    fold_sha256: String,
    orthogonality_sha256: String,
    selection: TellusSelectionV1,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TellusSelectionV1 {
    authority_sha256: String,
    surface_sha256: String,
    depth: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let database = args.next().ok_or("database path required")?;
    let tellus_facts = args.next().ok_or("Tellus facts path required")?;
    let output = args.next().ok_or("output path required")?;
    if args.next().is_some() || !database.is_absolute() || !output.is_absolute() {
        return Err("expected absolute DATABASE TELLUS_FACTS OUTPUT paths".into());
    }
    let facts: TellusFactsV1 = serde_json::from_slice(&fs::read(tellus_facts)?)?;
    if facts.version != 1 || facts.selection.depth != "ambient" {
        return Err("unsupported Tellus alignment facts".into());
    }

    let method = digest(&facts.method_sha256)?;
    let fold = digest(&facts.fold_sha256)?;
    let orthogonality = digest(&facts.orthogonality_sha256)?;
    let authority = digest(&facts.selection.authority_sha256)?;
    let surface = digest(&facts.selection.surface_sha256)?;

    let store = DeliberationSqliteStore::open(database, [31; 16], limits())?;
    let mut service = DeliberationService::new(store);
    apply(
        &mut service,
        0,
        MutationV1::Admit(Box::new(plan(
            method,
            fold,
            orthogonality,
            authority,
            surface,
        ))),
    )?;
    for (revision, report) in [
        (1, report(21, EXTERNAL_ADVOCATE)),
        (2, report(24, PRODUCT_MANAGER)),
        (3, report(27, FRONTIER_SUPERVISOR)),
    ] {
        apply(
            &mut service,
            revision,
            MutationV1::SubmitReport {
                deliberation: DeliberationIdV1(DELIBERATION),
                report,
            },
        )?;
    }
    apply(
        &mut service,
        4,
        MutationV1::Resolve {
            deliberation: DeliberationIdV1(DELIBERATION),
            resolution: ResolutionV1::Maybe(MaybeValueV1::Both),
        },
    )?;
    apply(
        &mut service,
        5,
        MutationV1::RequestHumanGate {
            deliberation: DeliberationIdV1(DELIBERATION),
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
                    expected_revision: 1,
                    expected_sha256: [44; 32],
                },
                requested_at: 36,
            },
        },
    )?;

    let output_value = serde_json::json!({
        "version": 1,
        "registry_revision": 6,
        "deliberation": lower_hex(DELIBERATION),
        "external_advocate": lower_hex(EXTERNAL_ADVOCATE),
        "product_manager": lower_hex(PRODUCT_MANAGER),
        "frontier_supervisor": lower_hex(FRONTIER_SUPERVISOR),
        "pending_human_gate": lower_hex([30; 16])
    });
    fs::write(output, serde_json::to_vec(&output_value)?)?;
    Ok(())
}

fn plan(
    method: [u8; 32],
    fold: [u8; 32],
    orthogonality: [u8; 32],
    authority: [u8; 32],
    surface: [u8; 32],
) -> DeliberationPlanV1 {
    let members = [
        (
            EXTERNAL_ADVOCATE,
            StakeholderRepresentationV1::ActualStakeholder,
            PersonaLensV1::Researcher,
        ),
        (
            PRODUCT_MANAGER,
            StakeholderRepresentationV1::AuthorizedRepresentative,
            PersonaLensV1::ConvergenceFacilitator,
        ),
        (
            FRONTIER_SUPERVISOR,
            StakeholderRepresentationV1::TechnicalSme,
            PersonaLensV1::RigorReviewer,
        ),
    ]
    .into_iter()
    .map(|(principal, representation, lens)| EnsembleMemberV1 {
        id: EnsembleMemberIdV1(principal),
        stakeholder: StakeholderRefV1 {
            principal,
            constituency_digest: [principal[0]; 32],
            representation,
            accountable_principal: match representation {
                StakeholderRepresentationV1::ActualStakeholder => Some(principal),
                StakeholderRepresentationV1::AuthorizedRepresentative => Some([50; 16]),
                StakeholderRepresentationV1::AgentModeledPerspective
                | StakeholderRepresentationV1::TechnicalSme => None,
            },
            mandate_digest: (representation
                == StakeholderRepresentationV1::AuthorizedRepresentative)
                .then_some([51; 32]),
            conflict_digest: Some([52; 32]),
        },
        lens,
        concern_root_digest: [principal[0].wrapping_add(1); 32],
        grounding: GroundingRefV1 {
            authority_digest: authority,
            surface_digest: surface,
            depth: SurfaceDepthV1::Ambient,
        },
    })
    .collect();

    DeliberationPlanV1 {
        id: DeliberationIdV1(DELIBERATION),
        question_digest: [20; 32],
        method: MethodRefV1 {
            machine_digest: method,
            state_digest: [3; 32],
        },
        grounding_provenance: Some(GroundingSourceProvenanceV1 {
            fold_content_digest: fold,
            orthogonality_content_digest: orthogonality,
        }),
        claims: vec![ClaimKindV1::Empirical, ClaimKindV1::Normative],
        criteria: vec![CriterionV1 {
            id: CriterionIdV1([4; 16]),
            direction: CriterionDirectionV1::Maximize,
        }],
        candidates: vec![CandidateV1 {
            id: CandidateIdV1([5; 16]),
            scores: vec![1],
        }],
        admission: EnsembleAdmissionV1::affirmed([12; 32], [13; 32], [14; 32], [15; 32], [16; 16]),
        members,
        orthogonality: OrthogonalityEvidenceV1 {
            query_digest: [10; 32],
            result_digest: [11; 32],
            all_orthogonal: true,
        },
        fuel: 8,
    }
}

fn report(id: u8, member: [u8; 16]) -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([id; 16]),
        member: EnsembleMemberIdV1(member),
        output_digest: [id; 32],
        evidence_digests: vec![[id.wrapping_add(1); 32]],
        dissent_digests: Vec::new(),
    }
}

fn apply(
    service: &mut DeliberationService<DeliberationSqliteStore>,
    expected_revision: u64,
    mutation: MutationV1,
) -> Result<(), Box<dyn Error>> {
    let reply = service.execute(CommandV1::Apply {
        expected_revision,
        mutation,
    })?;
    match reply {
        ReplyV1::Accepted(OutcomeV1::Applied {
            revision,
            replayed: false,
            ..
        }) if revision == expected_revision + 1 => Ok(()),
        _ => Err(format!(
            "Moira refused product-alignment fixture seed at revision {expected_revision}: {reply:?}"
        )
        .into()),
    }
}

fn limits() -> DeliberationStoreLimitsV1 {
    DeliberationStoreLimitsV1 {
        codec: CodecLimitsV1 {
            max_bytes: 65_536,
            max_deliberation_bytes: 32_768,
            registry: RegistryLimitsV1 {
                max_deliberations: 8,
                deliberation: DeliberationLimitsV1 {
                    max_claims: 8,
                    max_criteria: 8,
                    max_candidates: 8,
                    max_members: 4,
                    max_evidence_per_report: 8,
                    max_dissent_per_report: 8,
                    max_fuel: 32,
                },
            },
        },
        max_database_pages: 64,
    }
}

fn digest(value: &str) -> Result<[u8; 32], Box<dyn Error>> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("digest must be exactly 64 hexadecimal digits".into());
    }
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)?;
    }
    Ok(output)
}

fn lower_hex<const N: usize>(value: [u8; N]) -> String {
    let mut output = String::with_capacity(N * 2);
    for byte in value {
        use std::fmt::Write;
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}
