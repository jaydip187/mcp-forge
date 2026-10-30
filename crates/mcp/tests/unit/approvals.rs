use super::*;
use serde_json::json;

fn pending(id: Uuid) -> PendingApproval {
    let now = Utc::now();
    PendingApproval {
        id,
        project_id: Uuid::new_v4(),
        tool_id: Uuid::new_v4(),
        tool_name: "delete_user".into(),
        client: Some("claude-code 2.1".into()),
        arguments: json!({ "id": 42 }),
        created_at: now,
        expires_at: now + chrono::Duration::minutes(5),
        asked_client: false,
    }
}

#[tokio::test]
async fn first_answer_wins() {
    let a = Approvals::default();
    let id = Uuid::new_v4();
    let rx = a.open(pending(id));
    assert_eq!(a.list().len(), 1);
    assert!(a.get(id).is_some());

    assert!(a.decide(id, ApprovalDecision::Approved, Some("alice".into())));
    assert!(
        !a.decide(id, ApprovalDecision::Denied, None),
        "already answered"
    );
    assert_eq!(
        rx.await.unwrap(),
        Decision {
            decision: ApprovalDecision::Approved,
            by: Some("alice".into())
        }
    );
    assert!(a.list().is_empty());
}

#[tokio::test]
async fn cancel_all_answers_everyone() {
    let a = Approvals::default();
    let (x, y) = (Uuid::new_v4(), Uuid::new_v4());
    let (rx1, rx2) = (a.open(pending(x)), a.open(pending(y)));
    a.cancel_all();
    for rx in [rx1, rx2] {
        assert_eq!(rx.await.unwrap().decision, ApprovalDecision::Cancelled);
    }
    assert!(a.list().is_empty());
}

#[test]
fn deciding_after_the_waiter_left_is_harmless() {
    let a = Approvals::default();
    let id = Uuid::new_v4();
    drop(a.open(pending(id)));
    assert!(a.decide(id, ApprovalDecision::Denied, None));
}

#[test]
fn messages_tell_the_agent_what_happened() {
    let denied = Decision {
        decision: ApprovalDecision::Denied,
        by: Some("alice".into()),
    };
    assert!(not_approved_text(&denied, 300).contains("alice denied"));
    let timed_out = Decision {
        decision: ApprovalDecision::TimedOut,
        by: None,
    };
    assert!(not_approved_text(&timed_out, 300).contains("300 seconds"));
    assert!(Approvals::default().has_ui());
    assert!(!Approvals::without_ui().has_ui());
}
