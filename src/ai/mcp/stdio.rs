use anyhow::{Context, Result, anyhow};
use std::{collections::HashMap, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, oneshot},
};

use super::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

pub struct StdioTransport {
    _child: Child,
    request_tx: mpsc::Sender<(JsonRpcRequest, oneshot::Sender<Result<JsonRpcResponse>>)>,
    notification_tx: mpsc::Sender<JsonRpcNotification>,
}

impl StdioTransport {
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        for (k, v) in env {
            cmd.env(k, v);
        }

        #[cfg(windows)]
        {
            // Windows CREATE_NO_WINDOW flag to prevent console popping up
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn MCP process: {command}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to open stdin for MCP child process"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to open stdout for MCP child process"))?;

        let (request_tx, mut request_rx) =
            mpsc::channel::<(JsonRpcRequest, oneshot::Sender<Result<JsonRpcResponse>>)>(32);
        let (notification_tx, mut notification_rx) = mpsc::channel::<JsonRpcNotification>(32);

        // Background worker loop managing child stdin/stdout
        tokio::spawn(async move {
            let mut writer = stdin;
            let mut reader = BufReader::new(stdout).lines();
            let mut pending_requests: HashMap<u64, oneshot::Sender<Result<JsonRpcResponse>>> =
                HashMap::new();

            loop {
                tokio::select! {
                    Some((req, resp_sender)) = request_rx.recv() => {
                        let id = req.id;
                        match serde_json::to_string(&req) {
                            Ok(json_str) => {
                                if let Err(e) = writer.write_all(format!("{json_str}\n").as_bytes()).await {
                                    let _ = resp_sender.send(Err(anyhow!("Failed to write to MCP stdin: {e}")));
                                    continue;
                                }
                                if let Err(e) = writer.flush().await {
                                    let _ = resp_sender.send(Err(anyhow!("Failed to flush MCP stdin: {e}")));
                                    continue;
                                }
                                pending_requests.insert(id, resp_sender);
                            }
                            Err(e) => {
                                let _ = resp_sender.send(Err(anyhow!("Failed to serialize request: {e}")));
                            }
                        }
                    }
                    Some(notif) = notification_rx.recv() => {
                        if let Ok(json_str) = serde_json::to_string(&notif) {
                            let _ = writer.write_all(format!("{json_str}\n").as_bytes()).await;
                            let _ = writer.flush().await;
                        }
                    }
                    line_res = reader.next_line() => {
                        match line_res {
                            Ok(Some(line)) => {
                                let trimmed = line.trim();
                                if trimmed.is_empty() {
                                    continue;
                                }
                                if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(trimmed) {
                                    if let Some(id) = resp.id {
                                        if let Some(sender) = pending_requests.remove(&id) {
                                            let _ = sender.send(Ok(resp));
                                        }
                                    }
                                }
                            }
                            Ok(None) => {
                                // EOF on child stdout
                                for (_, sender) in pending_requests.drain() {
                                    let _ = sender.send(Err(anyhow!("MCP child process closed stdout")));
                                }
                                break;
                            }
                            Err(e) => {
                                tracing::error!("Error reading MCP stdout: {e}");
                                break;
                            }
                        }
                    }
                    else => break,
                }
            }
        });

        Ok(Self {
            _child: child,
            request_tx,
            notification_tx,
        })
    }

    pub async fn send_request(&self, req: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let (tx, rx) = oneshot::channel();
        self.request_tx
            .send((req, tx))
            .await
            .map_err(|_| anyhow!("Failed to send request to MCP worker"))?;
        rx.await
            .map_err(|_| anyhow!("MCP worker dropped response channel"))?
    }

    pub async fn send_notification(&self, notif: JsonRpcNotification) -> Result<()> {
        self.notification_tx
            .send(notif)
            .await
            .map_err(|_| anyhow!("Failed to send notification to MCP worker"))?;
        Ok(())
    }
}
