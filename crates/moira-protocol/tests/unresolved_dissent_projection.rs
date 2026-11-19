use moira_protocol::deliberation::{
    EnsembleMemberIdV1, MemberReportV1, ReportIdV1, UnresolvedDissentRefV1, unresolved_dissent,
};

fn report(id: u8, dissent_digests: Vec<[u8; 32]>) -> MemberReportV1 {
    MemberReportV1 {
        id: ReportIdV1([id; 16]),
        member: EnsembleMemberIdV1([id.wrapping_add(1); 16]),
        output_digest: [id.wrapping_add(2); 32],
        evidence_digests: vec![[id.wrapping_add(3); 32]],
        dissent_digests,
    }
}

#[test]
fn every_retained_report_dissent_is_explicitly_unresolved() {
    let reports = vec![
        report(1, vec![[11; 32], [12; 32]]),
        report(2, Vec::new()),
        report(3, vec![[13; 32]]),
    ];

    assert_eq!(
        unresolved_dissent(&reports).collect::<Vec<_>>(),
        vec![
            UnresolvedDissentRefV1 {
                source_report: ReportIdV1([1; 16]),
                dissent_digest: [11; 32],
            },
            UnresolvedDissentRefV1 {
                source_report: ReportIdV1([1; 16]),
                dissent_digest: [12; 32],
            },
            UnresolvedDissentRefV1 {
                source_report: ReportIdV1([3; 16]),
                dissent_digest: [13; 32],
            },
        ]
    );
}
