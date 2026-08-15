//! llama-server lifecycle: spawn, health-check, kill.
//!
//! One model per run, sequential rotation (RAM-safe). Server stderr goes to
//! `bench/tmp/<slug>-server.log` so load failures are diagnosable.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const PORT_FREE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct ServerMeta {
    /// Model id the server reports (used as the embedder `modelId`).
    pub model_id: String,
    /// Embedding dimension (`meta.n_embd`).
    pub dim: usize,
    /// Server context size, if reported.
    pub n_ctx: Option<u64>,
}

pub struct LlamaServer {
    child: Child,
    port: u16,
}

impl LlamaServer {
    /// Spawns llama-server in embedding mode and waits until `/v1/models`
    /// responds, returning the reported model metadata.
    pub async fn start(
        llama_server: &str,
        gguf: &Path,
        port: u16,
        log_file: &Path,
    ) -> anyhow::Result<(Self, ServerMeta)> {
        wait_port_free(port).await?;

        let log = std::fs::File::create(log_file)
            .with_context(|| format!("cannot create {}", log_file.display()))?;
        let child = Command::new(llama_server)
            .arg("-m")
            .arg(gguf)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("--embeddings")
            // Embedding inputs must fit in ONE physical batch; the 512
            // default rejects ~520-token inputs (e.g. package-lock.json's
            // long single lines), which silently drops whole batches for
            // some tokenizers. 4096 covers any single block we index.
            .arg("--batch-size")
            .arg("8192")
            .arg("--ubatch-size")
            .arg("4096")
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .spawn()
            .with_context(|| format!("failed to spawn {}", llama_server))?;

        let mut server = Self { child, port };
        match server.wait_model_meta().await {
            Ok(meta) => Ok((server, meta)),
            Err(err) => {
                server.kill();
                Err(err.context(format!(
                    "llama-server did not come up on port {port} (see {})",
                    log_file.display()
                )))
            }
        }
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    async fn wait_model_meta(&self) -> anyhow::Result<ServerMeta> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;
        let url = format!("http://127.0.0.1:{}/v1/models", self.port);
        let started = Instant::now();
        loop {
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let body: serde_json::Value = resp.json().await?;
                    let entry = &body["data"][0];
                    let dim = entry["meta"]["n_embd"].as_u64();
                    let dim = match dim {
                        Some(d) if d > 0 => d as usize,
                        _ => bail!("/v1/models did not report meta.n_embd — set \"dimension\" in models.json"),
                    };
                    let model_id = entry["id"]
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| "unknown".to_string());
                    let n_ctx = entry["meta"]["n_ctx"].as_u64();
                    return Ok(ServerMeta {
                        model_id,
                        dim,
                        n_ctx,
                    });
                }
                _ => {}
            }
            if started.elapsed() > STARTUP_TIMEOUT {
                bail!("startup timeout after {}s", STARTUP_TIMEOUT.as_secs());
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

impl Drop for LlamaServer {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Waits until nothing is listening on the port (previous server exited).
async fn wait_port_free(port: u16) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()?;
    let url = format!("http://127.0.0.1:{}/v1/models", port);
    let started = Instant::now();
    loop {
        // Connection refused means free; any HTTP response means occupied.
        if !matches!(client.get(&url).send().await, Ok(resp) if resp.status().is_success()) {
            return Ok(());
        }
        if started.elapsed() > PORT_FREE_TIMEOUT {
            bail!(
                "port {port} still occupied after {}s",
                PORT_FREE_TIMEOUT.as_secs()
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
