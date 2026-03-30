use crate::{git::pack, storage};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Write};

// --- 1. 定义符合 JSON-RPC 2.0 标准的结构 ---

#[derive(Deserialize, Debug)]
struct JsonRpcRequest {
    method: String,
    params: Option<Value>,
    id: Option<Value>,
}

#[derive(Serialize, Debug)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    id: Option<Value>,
}

#[derive(Serialize, Debug)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

/// MCP Server implementation for repository operations
pub struct RepoMcpServer;

impl RepoMcpServer {
    pub fn get_tools() -> Vec<Value> {
        vec![
            json!({
                "name": "list_repos",
                "description": "List all repositories with their details and refs",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }),
            json!({
                "name": "get_repo_details",
                "description": "Get detailed information about a specific repository",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "repo_id": {
                            "type": "string",
                            "description": "The ID of the repository"
                        }
                    },
                    "required": ["repo_id"]
                }
            }),
            json!({
                "name": "clone_repo",
                "description": "Clone a repository from its bundle to a local directory",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "repo_id": {
                            "type": "string",
                            "description": "The ID of the repository to clone"
                        },
                        "output_path": {
                            "type": "string",
                            "description": "The local path where the repository should be cloned"
                        }
                    },
                    "required": ["repo_id", "output_path"]
                }
            }),
        ]
    }

    pub async fn execute_tool(name: &str, args: Value) -> Result<Value> {
        match name {
            "list_repos" => Self::list_repos().await,
            "get_repo_details" => {
                let repo_id = args
                    .get("repo_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing repo_id parameter"))?;
                Self::get_repo_details(repo_id).await
            }
            "clone_repo" => {
                let repo_id = args
                    .get("repo_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing repo_id parameter"))?;
                let output_path = args
                    .get("output_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing output_path parameter"))?;
                Self::clone_repo(repo_id, output_path).await
            }
            _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
        }
    }

    async fn list_repos() -> Result<Value> {
        match storage::repo_model::list_repos().await {
            Ok(repos) => {
                let repo_list: Vec<Value> = repos
                    .iter()
                    .map(|repo| {
                        let mut repo_info = json!({
                            "repo_id": repo.repo_id,
                            "name": repo.p2p_description.name,
                            "creator": repo.p2p_description.creator,
                            "language": repo.p2p_description.language,
                            "size": repo.p2p_description.size,
                            "description": repo.p2p_description.description,
                            "path": repo.path.display().to_string(),
                            "bundle": repo.bundle.display().to_string(),
                            "latest_commit_at": repo.p2p_description.latest_commit_at,
                        });

                        // 恢复 refs 处理逻辑
                        if !repo.bundle.as_os_str().is_empty() {
                            if let Ok(local_refs) =
                                pack::extract_bundle_refs(&repo.bundle.to_string_lossy())
                            {
                                let refs: Vec<Value> = local_refs
                                    .iter()
                                    .map(|(ref_name, commit)| {
                                        json!({
                                            "name": ref_name,
                                            "commit": commit
                                        })
                                    })
                                    .collect();
                                repo_info["refs"] = Value::Array(refs);

                                let mut has_updates = false;
                                if !repo.path.as_os_str().is_empty() && repo.path.exists() {
                                    if let Ok(current_refs) = crate::git::git_repo::read_repo_refs(
                                        repo.path.to_str().unwrap_or(""),
                                    ) {
                                        has_updates = current_refs != local_refs;
                                    }
                                }
                                repo_info["has_updates"] = Value::Bool(has_updates);
                            }
                        }

                        repo_info
                    })
                    .collect();
                Ok(json!({
                   "content": [{
                       "type": "text",
                       "text": serde_json::to_string(&repo_list)?
                   }]
                }))
            }
            Err(e) => Err(e),
        }
    }

    async fn get_repo_details(repo_id: &str) -> Result<Value> {
        match storage::repo_model::load_repo_from_db(repo_id).await {
            Ok(Some(repo)) => {
                let mut repo_info = json!({
                    "repo_id": repo.repo_id,
                    "name": repo.p2p_description.name,
                    "creator": repo.p2p_description.creator,
                    "description": repo.p2p_description.description,
                    "path": repo.path.display().to_string(),
                    "bundle": repo.bundle.display().to_string(),
                    "latest_commit_at": repo.p2p_description.latest_commit_at,
                });

                // Check for updates if this is a local repo
                if !repo.path.as_os_str().is_empty() && repo.path.exists() {
                    if let Ok(current_refs) =
                        crate::git::git_repo::read_repo_refs(repo.path.to_str().unwrap_or(""))
                    {
                        if let Ok(local_refs) =
                            pack::extract_bundle_refs(&repo.bundle.to_string_lossy())
                        {
                            repo_info["has_updates"] = Value::Bool(current_refs != local_refs);

                            let local_ref_list: Vec<Value> = local_refs
                                .iter()
                                .map(|(ref_name, commit)| {
                                    json!({
                                        "name": ref_name,
                                        "commit": commit
                                    })
                                })
                                .collect();
                            repo_info["local_refs"] = Value::Array(local_ref_list);

                            let current_ref_list: Vec<Value> = current_refs
                                .iter()
                                .map(|(ref_name, commit)| {
                                    json!({
                                        "name": ref_name,
                                        "commit": commit
                                    })
                                })
                                .collect();
                            repo_info["current_refs"] = Value::Array(current_ref_list);
                        }
                    }
                }
                Ok(json!({
                   "content": [{
                       "type": "text",
                       "text": serde_json::to_string_pretty(&repo_info)?
                   }]
                }))
            }
            Ok(None) => Err(anyhow::anyhow!("Repository not found")),
            Err(e) => Err(e),
        }
    }

    async fn clone_repo(repo_id: &str, output: &str) -> Result<Value> {
        use std::path::PathBuf;
        match storage::repo_model::load_repo_from_db(repo_id).await {
            Ok(Some(mut repo)) => {
                let bundle_path = repo.bundle.to_string_lossy().to_string();
                if bundle_path.is_empty() || !std::path::Path::new(&bundle_path).exists() {
                    return Err(anyhow::anyhow!("Bundle file not found for repository"));
                }

                pack::restore_repo_from_bundle(&bundle_path, output).await?;

                // Read and save refs from the cloned repository
                if let Ok(refs) = crate::git::git_repo::read_repo_refs(output) {
                    let _ = storage::ref_model::batch_save_refs(repo_id, &refs).await;
                }

                // Update repo path to the cloned location
                repo.path = PathBuf::from(output);
                let _ = storage::repo_model::save_repo_to_db(&repo).await;

                Ok(json!({
                   "content": [{
                       "type": "text",
                       "text": format!("Successfully cloned repository {} to {}", repo_id, output)
                   }]
                }))
            }
            Ok(None) => Err(anyhow::anyhow!("Repository not found")),
            Err(e) => Err(e),
        }
    }
}

pub async fn start_mcp_server() -> Result<()> {
    eprintln!("MCP Repository Server started");

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());

    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        if line.trim().is_empty() {
            line.clear();
            continue;
        }

        // 1. 解析请求
        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to parse JSON: {}", e);
                line.clear();
                continue;
            }
        };

        eprintln!("Received method: {}", req.method);

        // 2. 处理请求并获取 Result 或 Error
        // 注意：这里返回元组 (Option<Result>, Option<Error>)
        let (result, error) = match req.method.as_str() {
            // A. 初始化握手 (必须响应 initialize)
            "initialize" => (
                Some(json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "megaengine-repo-mcp",
                        "version": "1.0"
                    }
                })),
                None,
            ),

            // B. 初始化完成通知 (不需要响应)
            "notifications/initialized" => {
                eprintln!("Client initialized.");
                line.clear();
                continue;
            }

            // C. 列出工具
            "tools/list" => {
                let tools = RepoMcpServer::get_tools();
                (Some(json!({ "tools": tools })), None)
            }

            // D. 调用工具
            "tools/call" => handle_tool_call(&req.params).await,

            // E. 心跳
            "ping" => (Some(json!({})), None),

            // F. 未知方法
            _ => (
                None,
                Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", req.method),
                    data: None,
                }),
            ),
        };

        // 3. 构建并发送响应
        if let Some(req_id) = req.id {
            let resp = JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result,
                error,
                id: Some(req_id),
            };

            let resp_str = serde_json::to_string(&resp)?;
            writeln!(stdout, "{}", resp_str)?;
            stdout.flush()?;
        }

        line.clear();
    }

    Ok(())
}

// 辅助函数：处理工具调用
async fn handle_tool_call(params: &Option<Value>) -> (Option<Value>, Option<JsonRpcError>) {
    let params = match params {
        Some(p) => p,
        None => {
            return (
                None,
                Some(JsonRpcError {
                    code: -32602,
                    message: "Missing params".into(),
                    data: None,
                }),
            )
        }
    };

    let name = match params.get("name").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => {
            return (
                None,
                Some(JsonRpcError {
                    code: -32602,
                    message: "Missing tool name".into(),
                    data: None,
                }),
            )
        }
    };

    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    // 调用业务逻辑
    match RepoMcpServer::execute_tool(name, args).await {
        Ok(res) => (Some(res), None), // 这里的 res 必须符合 CallToolResult 结构
        Err(e) => (
            None,
            Some(JsonRpcError {
                code: -32000, // 应用级错误
                message: e.to_string(),
                data: None,
            }),
        ),
    }
}
