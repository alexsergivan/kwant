use kwnt::broker::{ApprovalDecision, ApprovalRequest, PermissionBroker};
use serde_json::json;

#[tokio::test]
async fn test_permission_broker_allow_and_deny() {
    let broker = PermissionBroker::new();

    let req = ApprovalRequest {
        id: "ticket-1".into(),
        kwant_id: "k-1".into(),
        tool_name: "execute_shell".into(),
        details: json!({"command": "rm -rf /"}),
    };

    let rx = broker.request_approval(req).await;
    assert!(broker.is_pending("ticket-1").await);

    // Resolve in background
    let broker_clone = broker.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        broker_clone.resolve("ticket-1", ApprovalDecision::AllowOnce).await;
    });

    let decision = rx.await.unwrap();
    assert_eq!(decision, ApprovalDecision::AllowOnce);
    assert!(!broker.is_pending("ticket-1").await);
}
