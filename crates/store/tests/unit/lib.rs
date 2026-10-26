use super::*;

#[tokio::test]
async fn open_creates_db_file_and_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nested").join("data");
    let store = Store::open(&dir).await.unwrap();
    assert!(dir.join(DB_FILE).exists());
    store.ping().await.unwrap();
}

#[tokio::test]
async fn reopen_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    Store::open(tmp.path()).await.unwrap().pool().close().await;
    Store::open(tmp.path()).await.unwrap().ping().await.unwrap();
}

#[tokio::test]
async fn in_memory_works() {
    Store::open_in_memory().await.unwrap().ping().await.unwrap();
}
