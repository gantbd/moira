//! Optional trusted peer-effect composition for the methodical operator.

use super::{
    AegisResolutionConfigV1, ErrorV1, FixedTime, ProductConfigV1, StoreConfigV1, empty_path, id,
};
use aegis_client::AuthorityClient;
use aegis_protocol::{ActionV1, EngineNamespaceV1, OutcomeV1, ReplyV1, ScopeV1};
use moira_aegis::{AegisActionSelectorV1, choice_resource};
use moira_agora::{PublicationLimitsV1, durable::DurableResponsePublisher};
use moira_agora_local::{AgoraUnixLimitsV1, connect_agora_v2};
use moira_agora_publisher_sqlite::{PublisherSqliteStore, PublisherStoreLimitsV1};
use moira_client::{ModelSagaMutationClient, ModelSagaServicePort};
use moira_core::deliberation::{DeliberationCheckpointV1, EnsembleMemberIdV1};
use moira_daimon::ensemble::MemberOutcomeV1;
use moira_local_harness::{
    action_selection::{select_authorized, select_single_member},
    peer_router::{AgoraRouteV1, ExternalConversationRefV1, FederatedPeerRouterV1},
    peer_turn::{PeerTurnOutcomeV1, execute_selected_peer_turn, resume_peer_turn},
};
use moira_protocol::{WorkflowIdV1, model_saga as saga};
use moira_service::model_saga::ModelSagaService;
use moira_sqlite::model_saga::{ModelSagaSqliteStore, ModelSagaStoreLimitsV1};
use moira_vellum::{DocumentRouteV1, EditAdapter, ExternalDocumentRefV1};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};
use vellum_protocol::{WorkspaceDocumentIdV1, WorkspaceIdV1};
use vellum_transport::{
    AuthenticatedUnixConnector, LocalAuthenticationClaimV1, RemoteTransport, TransportLimits,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerActionConfigV1 {
    pub member: String,
    pub workflow: String,
    pub saga_store: PeerSagaStoreConfigV1,
    #[serde(default)]
    pub selection: PeerSelectionConfigV1,
    #[serde(default)]
    pub aegis: Option<AegisResolutionConfigV1>,
    pub agora: AgoraPeerConfigV1,
    pub vellum: VellumPeerConfigV1,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PeerSelectionConfigV1 {
    SingleMember {
        policy_digest: String,
    },
    #[default]
    Authorized,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerSagaStoreConfigV1 {
    pub path: PathBuf,
    pub owner: String,
    pub max_sagas: usize,
    pub max_proposals_per_saga: usize,
    pub max_proposal_bytes: usize,
    #[serde(default = "default_checkpoint_bytes")]
    pub max_checkpoint_bytes: usize,
    #[serde(default = "super::default_database_pages")]
    pub max_database_pages: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgoraPeerConfigV1 {
    pub socket: PathBuf,
    pub participant: String,
    pub publisher_store: StoreConfigV1,
    pub routes: Vec<AgoraRouteConfigV1>,
    #[serde(default = "super::default_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgoraRouteConfigV1 {
    pub room: String,
    pub reply_to: String,
    pub source: String,
    pub room_sequence: u64,
    pub room_id: String,
    pub expected_agora_revision: u64,
    pub logical_time: u64,
    #[serde(default)]
    pub addressed_to: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VellumPeerConfigV1 {
    pub socket: PathBuf,
    pub claim_env: String,
    pub routes: Vec<VellumRouteConfigV1>,
    pub max_replacement_bytes: usize,
    #[serde(default = "super::default_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VellumRouteConfigV1 {
    pub workspace: String,
    pub document: String,
    pub vellum_workspace: u64,
    pub vellum_document: u64,
    pub source_revision: u64,
    pub source_extent: u64,
}

#[derive(Debug)]
pub(crate) struct ValidatedPeerActionV1 {
    member: EnsembleMemberIdV1,
    workflow: WorkflowIdV1,
    saga_store: PeerSagaStoreConfigV1,
    saga_owner: [u8; 16],
    selection: ValidatedPeerSelectionV1,
    agora: AgoraPeerConfigV1,
    publisher_owner: [u8; 16],
    agora_routes: BTreeMap<ExternalConversationRefV1, AgoraRouteV1>,
    vellum: VellumPeerConfigV1,
    vellum_routes: BTreeMap<ExternalDocumentRefV1, DocumentRouteV1>,
    extents: BTreeMap<([u8; 16], [u8; 16], u64), u64>,
}

#[derive(Debug)]
enum ValidatedPeerSelectionV1 {
    SingleMember {
        policy_digest: [u8; 32],
    },
    Authorized {
        aegis: AegisResolutionConfigV1,
        subject: [u8; 16],
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerExecutionV1 {
    Completed,
    Failed,
    Uncertain,
}

/// Validate and execute one already-resolved trusted peer-action composition.
///
/// This is the embeddable seam used by isolated daemon qualification. The
/// one-shot operator calls the same validator and executor after its governed
/// model/tool loop, so qualification does not substitute a second route.
pub fn execute_config<E>(
    raw: PeerActionConfigV1,
    checkpoint: &DeliberationCheckpointV1,
    outcomes: &[MemberOutcomeV1<E>],
    deliberation_path: &Path,
    deliberation_owner: [u8; 16],
    now: u64,
) -> Result<PeerExecutionV1, ErrorV1> {
    let members = checkpoint
        .plan
        .members
        .iter()
        .map(|member| member.id)
        .collect::<BTreeSet<_>>();
    let authorized_choice = matches!(
        checkpoint.resolution,
        Some(moira_core::deliberation::ResolutionV1::AuthorizedChoice { .. })
    );
    let validated = validate(
        Some(raw),
        &members,
        if members.len() == 1 {
            ProductConfigV1::Chatbot
        } else {
            ProductConfigV1::Workflow
        },
        authorized_choice,
        deliberation_path,
        deliberation_owner,
        now,
    )?
    .ok_or(ErrorV1::InvalidConfig)?;
    execute(validated, checkpoint, outcomes, now)
}

struct PeerSagaPort(ModelSagaService<ModelSagaSqliteStore>);

impl ModelSagaServicePort for PeerSagaPort {
    type Error = moira_sqlite::model_saga::ModelSagaStoreErrorV1;

    fn execute(&mut self, command: saga::CommandV1) -> Result<saga::ReplyV1, Self::Error> {
        self.0.execute(command)
    }
}

pub(crate) fn validate(
    raw: Option<PeerActionConfigV1>,
    members: &BTreeSet<EnsembleMemberIdV1>,
    product: ProductConfigV1,
    authorized_choice: bool,
    deliberation_path: &Path,
    deliberation_owner: [u8; 16],
    now: u64,
) -> Result<Option<ValidatedPeerActionV1>, ErrorV1> {
    let Some(raw) = raw else { return Ok(None) };
    let member = EnsembleMemberIdV1(id(&raw.member)?);
    let workflow = WorkflowIdV1(id(&raw.workflow)?);
    let saga_owner = id(&raw.saga_store.owner)?;
    let publisher_owner = id(&raw.agora.publisher_store.owner)?;
    let selection = match (product, raw.selection, raw.aegis) {
        (ProductConfigV1::Chatbot, PeerSelectionConfigV1::SingleMember { policy_digest }, None)
            if members.len() == 1 =>
        {
            ValidatedPeerSelectionV1::SingleMember {
                policy_digest: digest(&policy_digest)?,
            }
        }
        (
            ProductConfigV1::Workflow | ProductConfigV1::Agent,
            PeerSelectionConfigV1::Authorized,
            Some(aegis),
        ) if authorized_choice
            && !empty_path(&aegis.socket)
            && (1..=300_000).contains(&aegis.timeout_ms)
            && aegis.expires_at > now =>
        {
            let subject = id(&aegis.subject)?;
            ValidatedPeerSelectionV1::Authorized { aegis, subject }
        }
        _ => return Err(ErrorV1::InvalidConfig),
    };
    let env_ok = !raw.vellum.claim_env.is_empty()
        && raw.vellum.claim_env.len() <= 256
        && raw
            .vellum
            .claim_env
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if !members.contains(&member)
        || empty_path(&raw.saga_store.path)
        || empty_path(&raw.agora.socket)
        || empty_path(&raw.agora.publisher_store.path)
        || empty_path(&raw.vellum.socket)
        || raw.agora.participant.is_empty()
        || raw.agora.participant.len() > 256
        || raw.saga_store.max_sagas == 0
        || raw.saga_store.max_proposals_per_saga == 0
        || raw.saga_store.max_proposal_bytes == 0
        || !(1_024..=64 * 1_048_576).contains(&raw.saga_store.max_checkpoint_bytes)
        || !(64..=1_048_576).contains(&raw.saga_store.max_database_pages)
        || !(64..=1_048_576).contains(&raw.agora.publisher_store.max_database_pages)
        || raw.vellum.max_replacement_bytes == 0
        || !(1..=300_000).contains(&raw.agora.timeout_ms)
        || !(1..=300_000).contains(&raw.vellum.timeout_ms)
        || !env_ok
        || [saga_owner, publisher_owner, deliberation_owner]
            .into_iter()
            .collect::<BTreeSet<_>>()
            .len()
            != 3
        || [
            &raw.saga_store.path,
            &raw.agora.publisher_store.path,
            deliberation_path,
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
        .len()
            != 3
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let mut agora_routes = BTreeMap::new();
    for route in &raw.agora.routes {
        if route.room_sequence == 0
            || route.room_id.is_empty()
            || route.expected_agora_revision == 0
            || route.addressed_to.iter().any(String::is_empty)
            || route.addressed_to.iter().collect::<BTreeSet<_>>().len() != route.addressed_to.len()
        {
            return Err(ErrorV1::InvalidConfig);
        }
        let key = ExternalConversationRefV1 {
            room: id(&route.room)?,
            reply_to: id(&route.reply_to)?,
        };
        let value = AgoraRouteV1 {
            source: id(&route.source)?,
            room_sequence: route.room_sequence,
            room_id: route.room_id.clone(),
            expected_agora_revision: route.expected_agora_revision,
            logical_time: route.logical_time,
            addressed_to: route.addressed_to.clone(),
        };
        if agora_routes.insert(key, value).is_some() {
            return Err(ErrorV1::InvalidConfig);
        }
    }
    let mut vellum_routes = BTreeMap::new();
    let mut extents = BTreeMap::new();
    for route in &raw.vellum.routes {
        if route.source_revision == 0 {
            return Err(ErrorV1::InvalidConfig);
        }
        let external = ExternalDocumentRefV1 {
            workspace: id(&route.workspace)?,
            document: id(&route.document)?,
        };
        let workspace = WorkspaceIdV1::new(route.vellum_workspace).ok_or(ErrorV1::InvalidConfig)?;
        let document = WorkspaceDocumentIdV1::new(workspace, route.vellum_document)
            .ok_or(ErrorV1::InvalidConfig)?;
        if vellum_routes
            .insert(
                external,
                DocumentRouteV1 {
                    workspace,
                    document,
                },
            )
            .is_some()
            || extents
                .insert(
                    (external.workspace, external.document, route.source_revision),
                    route.source_extent,
                )
                .is_some()
        {
            return Err(ErrorV1::InvalidConfig);
        }
    }
    Ok(Some(ValidatedPeerActionV1 {
        member,
        workflow,
        saga_store: raw.saga_store,
        saga_owner,
        selection,
        publisher_owner,
        agora_routes,
        vellum_routes,
        extents,
        agora: raw.agora,
        vellum: raw.vellum,
    }))
}

pub(crate) fn execute<E>(
    config: ValidatedPeerActionV1,
    checkpoint: &DeliberationCheckpointV1,
    outcomes: &[MemberOutcomeV1<E>],
    now: u64,
) -> Result<PeerExecutionV1, ErrorV1> {
    let limits = moira_core::model_saga::LimitsV1 {
        max_sagas: config.saga_store.max_sagas,
        max_proposals_per_saga: config.saga_store.max_proposals_per_saga,
        max_proposal_bytes: config.saga_store.max_proposal_bytes,
    };
    let store = ModelSagaSqliteStore::open(
        &config.saga_store.path,
        config.saga_owner,
        ModelSagaStoreLimitsV1 {
            codec: moira_core::model_saga::persistence::CodecLimitsV1 {
                max_bytes: config.saga_store.max_checkpoint_bytes,
                max_sagas: limits.max_sagas,
                max_proposals_per_saga: limits.max_proposals_per_saga,
                max_proposal_bytes: limits.max_proposal_bytes,
            },
            saga: limits,
            max_database_pages: config.saga_store.max_database_pages,
        },
    )
    .map_err(|_| ErrorV1::Peer)?;
    let expected = store.storage_revision().map(|revision| revision.0);
    let retained = store
        .saga(moira_core::WorkflowId::new(config.workflow.0))
        .map(|snapshot| snapshot.phase);
    if matches!(retained, Some(moira_core::model_saga::PhaseV1::Completed)) {
        return Ok(PeerExecutionV1::Completed);
    }
    if matches!(
        retained,
        Some(moira_core::model_saga::PhaseV1::Failed { .. })
    ) {
        return Ok(PeerExecutionV1::Failed);
    }
    let mut saga_port = PeerSagaPort(ModelSagaService::new(store));

    let mut selected = None;
    if retained.is_none() {
        let outcome = outcomes.iter().find(|outcome| matches!(outcome, MemberOutcomeV1::Report { member, .. } if *member == config.member)).ok_or(ErrorV1::Peer)?;
        selected = Some(match config.selection {
            ValidatedPeerSelectionV1::SingleMember { policy_digest } => {
                select_single_member::<E, ()>(checkpoint, outcome, policy_digest)
                    .map_err(|_| ErrorV1::Peer)?
            }
            ValidatedPeerSelectionV1::Authorized { aegis, subject } => {
                let (session, port) = aegis_local_daemon::unix::connect(
                    &aegis.socket,
                    aegis_wire::LimitsV1::default(),
                    Duration::from_millis(aegis.timeout_ms),
                )
                .map_err(|_| ErrorV1::Peer)?;
                let mut authority = AuthorityClient::new(session, port);
                let ReplyV1::Accepted(OutcomeV1::GrantIssued(grant)) = authority
                    .issue(
                        aegis.expected_epoch,
                        now,
                        aegis.expires_at,
                        vec![ScopeV1 {
                            engine: EngineNamespaceV1::Moira,
                            resource: choice_resource(checkpoint.plan.id),
                            actions: 1 << ActionV1::Administer as u8,
                        }],
                    )
                    .map_err(|_| ErrorV1::Peer)?
                else {
                    return Err(ErrorV1::Peer);
                };
                let mut selector =
                    AegisActionSelectorV1::new(authority, grant.handle, subject, FixedTime(now));
                select_authorized(checkpoint, outcome, &mut selector).map_err(|_| ErrorV1::Peer)?
            }
        });
    }

    let (client, agora_port) = connect_agora_v2(
        &config.agora.socket,
        &config.agora.participant,
        AgoraUnixLimitsV1 {
            timeout: Duration::from_millis(config.agora.timeout_ms),
            ..AgoraUnixLimitsV1::default()
        },
    )
    .map_err(|_| ErrorV1::Peer)?;
    let publisher_store = PublisherSqliteStore::open(
        &config.agora.publisher_store.path,
        config.publisher_owner,
        client,
        PublisherStoreLimitsV1 {
            max_database_pages: config.agora.publisher_store.max_database_pages,
            ..PublisherStoreLimitsV1::default()
        },
    )
    .map_err(|_| ErrorV1::Peer)?;
    let publisher =
        DurableResponsePublisher::new(publisher_store, agora_port, PublicationLimitsV1::default())
            .map_err(|_| ErrorV1::Peer)?;

    let claim = std::env::var(&config.vellum.claim_env).map_err(|_| ErrorV1::MissingSecret)?;
    let claim = LocalAuthenticationClaimV1::new(id(&claim).map_err(|_| ErrorV1::MissingSecret)?);
    let connector = AuthenticatedUnixConnector::with_timeout(
        config.vellum.socket,
        claim,
        Duration::from_millis(config.vellum.timeout_ms),
    )
    .map_err(|_| ErrorV1::Peer)?;
    let transport =
        RemoteTransport::new(connector, TransportLimits::reference()).map_err(|_| ErrorV1::Peer)?;
    let vellum = EditAdapter::connect(
        transport,
        config.vellum_routes,
        config.vellum.max_replacement_bytes,
    )
    .map_err(|_| ErrorV1::Peer)?;
    let mut router = FederatedPeerRouterV1::new(publisher, config.agora_routes, vellum);
    let extents = config.extents;
    let result = if let Some(selected) = selected {
        let mut saga_client = ModelSagaMutationClient::new(saga_port);
        execute_selected_peer_turn(
            &mut saga_client,
            &mut router,
            expected,
            config.workflow,
            selected,
            |workspace, document, revision| {
                extents
                    .get(&(workspace, document, revision))
                    .copied()
                    .ok_or(())
            },
        )
        .map_err(|_| ErrorV1::Peer)?
    } else {
        resume_peer_turn(&mut saga_port, &mut router, config.workflow).map_err(|_| ErrorV1::Peer)?
    };
    let (publisher, _, _) = router.into_parts();
    let (_, agora_port) = publisher.into_parts();
    if let Some(port) = agora_port {
        let _ = port.disconnect();
    }
    Ok(match result {
        PeerTurnOutcomeV1::Completed { .. } => PeerExecutionV1::Completed,
        PeerTurnOutcomeV1::Failed { .. } => PeerExecutionV1::Failed,
        PeerTurnOutcomeV1::Uncertain { .. } => PeerExecutionV1::Uncertain,
    })
}

const fn default_checkpoint_bytes() -> usize {
    1_048_576
}

fn digest(value: &str) -> Result<[u8; 32], ErrorV1> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ErrorV1::InvalidConfig);
    }
    let mut result = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        result[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    if result == [0; 32] {
        return Err(ErrorV1::InvalidConfig);
    }
    Ok(result)
}

fn nibble(value: u8) -> Result<u8, ErrorV1> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(ErrorV1::InvalidConfig),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moira_core::deliberation::{
        CandidateIdV1, CandidateV1, ClaimKindV1, CriterionDirectionV1, CriterionIdV1, CriterionV1,
        DeliberationIdV1, DeliberationLimitsV1, DeliberationPlanV1, DeliberationV1,
        EnsembleMemberV1, GroundingRefV1, MemberReportV1, MethodRefV1, OrthogonalityEvidenceV1,
        PersonaLensV1, ReportIdV1, SurfaceDepthV1,
    };
    use moira_daimon::{
        ModelResultV1,
        ensemble::{MemberOutcomeV1, MemberResponseV1},
    };
    use moira_protocol::{DispositionV1, EffectIdV1, ResultReceiptV1, TaskRefV1};
    use sha2::{Digest, Sha256};

    #[test]
    fn completed_peer_saga_replays_without_authority_or_peer_connections() {
        let directory = std::env::temp_dir().join(format!(
            "moira-methodical-peer-replay-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        let workflow = WorkflowIdV1([40; 16]);
        let limits = moira_core::model_saga::LimitsV1 {
            max_sagas: 2,
            max_proposals_per_saga: 2,
            max_proposal_bytes: 4_096,
        };
        let store = ModelSagaSqliteStore::open(
            directory.join("saga.db"),
            [41; 16],
            ModelSagaStoreLimitsV1 {
                codec: moira_core::model_saga::persistence::CodecLimitsV1 {
                    max_bytes: 64 * 1_024,
                    max_sagas: 2,
                    max_proposals_per_saga: 2,
                    max_proposal_bytes: 4_096,
                },
                saga: limits,
                max_database_pages: 64,
            },
        )
        .unwrap();
        let mut client = ModelSagaMutationClient::new(PeerSagaPort(ModelSagaService::new(store)));
        let reply = client
            .apply(
                None,
                saga::MutationV1::Admit {
                    workflow,
                    daimon_effect: EffectIdV1([42; 16]),
                    selection_digest: [43; 32],
                    proposals: Vec::new(),
                },
            )
            .unwrap();
        assert!(matches!(
            reply,
            saga::ReplyV1::Accepted(saga::OutcomeV1::Applied {
                saga: saga::SagaViewV1 {
                    phase: saga::PhaseV1::Completed,
                    ..
                },
                ..
            })
        ));
        drop(client);

        let (checkpoint, _) = retained_report();
        let config = ValidatedPeerActionV1 {
            member: EnsembleMemberIdV1([5; 16]),
            workflow,
            saga_store: PeerSagaStoreConfigV1 {
                path: directory.join("saga.db"),
                owner: "29292929292929292929292929292929".into(),
                max_sagas: 2,
                max_proposals_per_saga: 2,
                max_proposal_bytes: 4_096,
                max_checkpoint_bytes: 64 * 1_024,
                max_database_pages: 64,
            },
            saga_owner: [41; 16],
            selection: ValidatedPeerSelectionV1::Authorized {
                aegis: AegisResolutionConfigV1 {
                    socket: directory.join("absent-aegis.sock"),
                    subject: "2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c".into(),
                    expected_epoch: 0,
                    expires_at: 20,
                    timeout_ms: 1,
                },
                subject: [44; 16],
            },
            agora: AgoraPeerConfigV1 {
                socket: directory.join("absent-agora.sock"),
                participant: "moira".into(),
                publisher_store: StoreConfigV1 {
                    path: directory.join("publisher.db"),
                    owner: "2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d".into(),
                    max_database_pages: 64,
                },
                routes: Vec::new(),
                timeout_ms: 1,
            },
            publisher_owner: [45; 16],
            agora_routes: BTreeMap::new(),
            vellum: VellumPeerConfigV1 {
                socket: directory.join("absent-vellum.sock"),
                claim_env: "DEFINITELY_ABSENT_MOIRA_TEST_SECRET".into(),
                routes: Vec::new(),
                max_replacement_bytes: 1,
                timeout_ms: 1,
            },
            vellum_routes: BTreeMap::new(),
            extents: BTreeMap::new(),
        };
        assert_eq!(
            execute::<()>(config, &checkpoint, &[], 10),
            Ok(PeerExecutionV1::Completed)
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn retained_report() -> (DeliberationCheckpointV1, MemberOutcomeV1<()>) {
        let member = EnsembleMemberIdV1([5; 16]);
        let plan = DeliberationPlanV1 {
            id: DeliberationIdV1([1; 16]),
            question_digest: [2; 32],
            method: MethodRefV1 {
                machine_digest: [3; 32],
                state_digest: [4; 32],
            },
            grounding_provenance: None,
            claims: vec![ClaimKindV1::Empirical],
            criteria: vec![CriterionV1 {
                id: CriterionIdV1([6; 16]),
                direction: CriterionDirectionV1::Maximize,
            }],
            candidates: vec![CandidateV1 {
                id: CandidateIdV1([7; 16]),
                scores: vec![1],
            }],
            admission: moira_core::deliberation::EnsembleAdmissionV1::affirmed(
                [13; 32], [14; 32], [15; 32], [16; 32], [17; 16],
            ),
            members: vec![EnsembleMemberV1 {
                id: member,
                stakeholder: moira_core::deliberation::StakeholderRefV1::technical_sme(
                    member.0, [8; 32],
                ),
                lens: PersonaLensV1::Researcher,
                concern_root_digest: [8; 32],
                grounding: GroundingRefV1 {
                    authority_digest: [9; 32],
                    surface_digest: [10; 32],
                    depth: SurfaceDepthV1::Ambient,
                },
            }],
            orthogonality: OrthogonalityEvidenceV1 {
                query_digest: [11; 32],
                result_digest: [12; 32],
                all_orthogonal: true,
            },
            fuel: 2,
        };
        let output = b"answer".to_vec();
        let report = MemberReportV1 {
            id: ReportIdV1([13; 16]),
            member,
            output_digest: Sha256::digest(&output).into(),
            evidence_digests: Vec::new(),
            dissent_digests: Vec::new(),
        };
        let mut deliberation =
            DeliberationV1::try_new(plan, DeliberationLimitsV1::default()).unwrap();
        deliberation.submit_report(report.clone()).unwrap();
        let outcome = MemberOutcomeV1::Report {
            member,
            report,
            response: MemberResponseV1 { claims: Vec::new() },
            result: Box::new(ModelResultV1 {
                receipt: ResultReceiptV1 {
                    effect: EffectIdV1([42; 16]),
                    execution_id: [14; 16],
                    task: TaskRefV1 {
                        external_task_key: [15; 16],
                        incarnation: 1,
                    },
                    attempt_number: 1,
                    disposition: DispositionV1::Completed,
                },
                evidence_digest: [16; 32],
                invocation_evidence: None,
                input_tokens: 1,
                output_tokens: 1,
                output,
                proposals: Vec::new(),
            }),
        };
        (deliberation.checkpoint(), outcome)
    }
}
