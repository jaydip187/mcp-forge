use forge_store::{Store, DB_FILE};
use mcp_forge::AppState;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

#[tokio::test]
async fn serve_creates_db_and_answers_healthz() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let store = Store::open(&data_dir).await.unwrap();
    assert!(data_dir.join(DB_FILE).exists());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel::<()>();
    let server = tokio::spawn(mcp_forge::serve(listener, AppState { store }, async {
        let _ = rx.await;
    }));

    let resp = reqwest::get(format!("http://{addr}/healthz"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");

    let missing = reqwest::get(format!("http://{addr}/nope")).await.unwrap();
    assert_eq!(missing.status(), 404);

    tx.send(()).unwrap();
    server.await.unwrap().unwrap();
}
