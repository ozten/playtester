//! Shared helpers for server integration tests.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use playtest_server::ServerConfig;
use tempfile::TempDir;

/// A running server bound to an ephemeral port.
#[allow(dead_code)]
pub struct SpawnedServer {
    pub base_url: String,
    pub data_dir: PathBuf,
    _temp: TempDir,
    handle: tokio::task::JoinHandle<()>,
}

impl SpawnedServer {
    pub async fn start() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let data_dir = temp.path().to_path_buf();

        // Bind to :0 to grab an unused port from the OS.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral");
        let local_addr: SocketAddr = listener.local_addr().expect("local_addr");
        drop(listener);

        let cfg = ServerConfig {
            bind: local_addr,
            data_dir: data_dir.clone(),
        };

        let handle = tokio::spawn(async move {
            let _ = playtest_server::run(cfg).await;
        });

        // Poll the health endpoint until the server is up.
        let base_url = format!("http://{local_addr}");
        let client = reqwest::Client::new();
        for _ in 0..100 {
            if let Ok(resp) = client
                .get(format!("{base_url}/api/health"))
                .timeout(Duration::from_millis(200))
                .send()
                .await
                && resp.status().is_success()
            {
                return Self {
                    base_url,
                    data_dir,
                    _temp: temp,
                    handle,
                };
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("server failed to start within timeout");
    }

    pub fn shutdown(self) {
        self.handle.abort();
    }
}
