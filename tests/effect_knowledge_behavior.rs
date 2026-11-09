use moira_core::EffectKnowledgeV1;

#[test]
fn contradictory_effect_evidence_fences_automatic_progress() {
    assert_eq!(
        EffectKnowledgeV1::Completed.join(EffectKnowledgeV1::Failed),
        EffectKnowledgeV1::Conflict
    );
    assert!(!EffectKnowledgeV1::Conflict.leq(EffectKnowledgeV1::Completed));
}
