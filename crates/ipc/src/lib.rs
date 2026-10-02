//! Optional latest-snapshot stream over a Unix domain socket; no database or REST.
//! Version 1: newline-delimited JSON; watch coalesces updates for slow readers.
#[cfg(unix)]
pub async fn serve(core: physio_core::RealtimeCore, path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::UnixListener::bind(path)?;
    let _guard = SocketGuard::new(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut receiver = core.subscribe();
        tokio::spawn(async move {
            loop {
                let snapshot = receiver.borrow_and_update().clone();
                let Ok(mut bytes) = serde_json::to_vec(&snapshot) else {
                    break;
                };
                bytes.push(b'\n');
                if !matches!(
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        stream.write_all(&bytes)
                    )
                    .await,
                    Ok(Ok(()))
                ) {
                    break;
                }
                if receiver.changed().await.is_err() {
                    break;
                }
            }
        });
    }
}

#[cfg(unix)]
struct SocketGuard {
    path: std::path::PathBuf,
    inode: u64,
}
#[cfg(unix)]
impl SocketGuard {
    fn new(path: &std::path::Path) -> anyhow::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            path: path.into(),
            inode: std::fs::metadata(path)?.ino(),
        })
    }
}
#[cfg(unix)]
impl Drop for SocketGuard {
    fn drop(&mut self) {
        use std::os::unix::fs::MetadataExt;
        if std::fs::symlink_metadata(&self.path).is_ok_and(|m| m.ino() == self.inode) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
