use aegis_authority::{
    ActionSetV1, ActionV1, AuthorityEpoch, AuthorizationRequestV1, Engine as Aegis, EngineId,
    EngineNamespaceV1, GrantDescriptorV1, Limits as AegisLimits, LogicalTime as AuthorityTime,
    PrincipalId as AuthorityPrincipal, ResourceId, ResourceScopeV1,
};
use daimon_harness::{
    AuthorityEvidenceV1, Engine as Daimon, ExecutionBudgetV1, ExecutionDispositionV1, ExecutionId,
    HarnessProfileId, HarnessRequestV1, IdempotencyKey, Limits as DaimonLimits, LogicalTime,
    PrincipalId, ProviderReceiptV1, TaskAttemptRefV1, ToolCapabilityId,
};
use moira_core::{
    AgoraMessageRefV1, ChronosAdmissionReceiptV1, ChronosAttemptReceiptV1, ChronosTaskRefV1,
    CommandV1, DaimonDispositionV1, DaimonResultReceiptV1, Engine as Moira, Limits as MoiraLimits,
    ParticipantRefV1, TransitionV1, WorkflowPhaseV1,
};

#[test]
fn daimon_uncertainty_is_attributed_once_and_fences_moira_replay() {
    let mut moira = Moira::try_new(MoiraLimits::default()).unwrap();
    let TransitionV1::ChronosSubmissionPrepared {
        workflow,
        effect_id: submission_effect,
    } = moira
        .apply(
            moira.revision(),
            CommandV1::AdmitMessage {
                message: AgoraMessageRefV1::new([1; 16], 2, 3),
                recipient: ParticipantRefV1::new([4; 16]),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    let task = ChronosTaskRefV1::new([5; 16], 6);
    moira
        .apply(
            moira.revision(),
            CommandV1::AcceptChronosAdmission {
                workflow,
                receipt: ChronosAdmissionReceiptV1::new(submission_effect, task, 7),
            },
        )
        .unwrap();
    let TransitionV1::DaimonExecutionPrepared {
        effect_id,
        attempt_number,
        ..
    } = moira
        .apply(
            moira.revision(),
            CommandV1::AcceptChronosAttempt {
                workflow,
                receipt: ChronosAttemptReceiptV1::new(task, 1, 8),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };

    let execution_bytes = effect_id.into_bytes();
    let execution_id = ExecutionId::new(execution_bytes);
    let principal = PrincipalId::new([9; 16]);
    let tool = ToolCapabilityId::new([10; 16]);
    let request = HarnessRequestV1::new(
        execution_id,
        TaskAttemptRefV1::new([5; 16], 6, attempt_number),
        principal,
        HarnessProfileId::new([11; 16]),
        IdempotencyKey::new(execution_bytes),
        vec![],
        vec![tool],
        ExecutionBudgetV1 {
            max_elapsed_millis: 1_000,
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_tool_calls: 1,
            max_context_bytes: 1_024,
            max_output_bytes: 1_024,
        },
    );
    let authority_engine_id = EngineId::new([12; 16]);
    let authority_subject = AuthorityPrincipal::new([9; 16]);
    let mut aegis = Aegis::try_new(authority_engine_id, AegisLimits::default()).unwrap();
    let grant = aegis
        .issue(
            GrantDescriptorV1::new(
                authority_subject,
                None,
                authority_engine_id,
                AuthorityEpoch(0),
                AuthorityTime(20),
                vec![ResourceScopeV1::new(
                    EngineNamespaceV1::Daimon,
                    ResourceId::new([10; 16]),
                    ActionSetV1::singleton(ActionV1::Execute),
                )],
            ),
            AuthorityTime(1),
        )
        .unwrap();
    let decision = aegis
        .validate(
            &grant,
            &AuthorizationRequestV1::new(
                authority_subject,
                None,
                EngineNamespaceV1::Daimon,
                ResourceId::new([10; 16]),
                ActionV1::Execute,
            ),
            AuthorityTime(2),
        )
        .unwrap();
    let authority = AuthorityEvidenceV1::new(
        decision.decision_id_bytes(),
        principal,
        [tool],
        LogicalTime(decision.expires_at().0),
    );
    let mut daimon = Daimon::try_new(DaimonLimits::default()).unwrap();
    let prepared = daimon
        .prepare(request.clone(), &authority, LogicalTime(10))
        .unwrap();
    assert_eq!(
        daimon
            .prepare(request, &authority, LogicalTime(10))
            .unwrap(),
        prepared
    );
    assert_eq!(daimon.in_flight(), 1);

    let provider_receipt =
        ProviderReceiptV1::new(execution_id, ExecutionDispositionV1::Uncertain, 0, vec![]);
    let result = daimon
        .dispatch(prepared.clone(), provider_receipt.clone())
        .unwrap();
    assert_eq!(daimon.dispatch(prepared, provider_receipt).unwrap(), result);
    assert_eq!(daimon.completed(), 1);

    let receipt = DaimonResultReceiptV1::new(
        effect_id,
        execution_bytes,
        task,
        attempt_number,
        match result.disposition {
            ExecutionDispositionV1::Completed => DaimonDispositionV1::Completed,
            ExecutionDispositionV1::Failed => DaimonDispositionV1::Failed,
            ExecutionDispositionV1::Cancelled => DaimonDispositionV1::Cancelled,
            ExecutionDispositionV1::TimedOut => DaimonDispositionV1::TimedOut,
            ExecutionDispositionV1::Uncertain => DaimonDispositionV1::Uncertain,
        },
    );
    let first = moira
        .apply(
            moira.revision(),
            CommandV1::AcceptDaimonResult { workflow, receipt },
        )
        .unwrap();
    let revision = moira.revision();
    assert_eq!(
        moira.workflow(&workflow).unwrap().phase(),
        WorkflowPhaseV1::FencedUncertain
    );
    assert_eq!(
        moira
            .apply(
                revision,
                CommandV1::AcceptDaimonResult { workflow, receipt },
            )
            .unwrap(),
        first
    );
    assert_eq!(moira.revision(), revision);
}
