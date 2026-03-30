use crate::mcp::mcp_server::RepoMcpServer;
use axum::{
    extract::{Query, State},
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    routing::{get, post},
    http::Method,
    Json, Router,
};
use futures::stream::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::{mpsc, RwLock};
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

// App state to hold active sessions
struct AppState {
    sessions: RwLock<HashMap<String, mpsc::UnboundedSender<Result<Event, axum::Error>>>>,
}

#[derive(Deserialize)]
struct SessionParam {
    session_id: String,
}

struct SessionCleanup {
    state: Arc<AppState>,
    session_id: String,
}

impl SessionCleanup {
    fn new(state: Arc<AppState>, session_id: String) -> Self {
        Self { state, session_id }
    }
}

impl Drop for SessionCleanup {
    fn drop(&mut self) {
        let state = Arc::clone(&self.state);
        let session_id = self.session_id.clone();

        tokio::spawn(async move {
            let removed = state.sessions.write().await.remove(&session_id);
            if removed.is_some() {
                tracing::info!("SSE session cleaned up: {}", session_id);
            }
        });
    }
}

pub async fn start_sse_server(addr: SocketAddr) -> anyhow::Result<()> {
    let state = Arc::new(AppState {
        sessions: RwLock::new(HashMap::new()),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any);

    let app = Router::new()
        .route("/sse", get(sse_handler))
        .route("/messages", post(message_handler))
        .with_state(state)
        .layer(cors);

    tracing::info!("MCP SSE Server listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn sse_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, axum::Error>>> {
    let session_id = Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::unbounded_channel();

    // Store the sender
    state
        .sessions
        .write()
        .await
        .insert(session_id.clone(), tx.clone());

    let stream = futures::stream::unfold(
        (rx, SessionCleanup::new(state.clone(), session_id.clone())),
        |(mut rx, cleanup)| async move { rx.recv().await.map(|event| (event, (rx, cleanup))) },
    );

    // Send the endpoint event immediately
    let endpoint_url = format!("/messages?session_id={}", session_id);
    let _ = tx.send(Ok(Event::default().event("endpoint").data(endpoint_url)));

    tracing::info!("New SSE session connected: {}", session_id);

    Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn message_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SessionParam>,
    Json(request): Json<Value>,
) -> impl IntoResponse {
    let session_id = params.session_id;

    let tx = {
        let sessions = state.sessions.read().await;
        sessions.get(&session_id).cloned()
    };

    if let Some(tx) = tx {
        // Handle the MCP request (JSON-RPC)
        // We spawn a task to process it so we don't block
        tokio::spawn(async move {
            if let Some(method) = request.get("method").and_then(|v| v.as_str()) {
                let response = match method {
                    "initialize" => Some(json!({
                        "jsonrpc": "2.0",
                        "id": request.get("id"),
                        "result": {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {
                                "tools": {}
                            },
                            "serverInfo": {
                                "name": "megaengine",
                                "version": "0.1.0"
                            }
                        }
                    })),
                    "notifications/initialized" => {
                        // Client initialized, no response needed for notification
                        None
                    }
                    "ping" => Some(json!({
                        "jsonrpc": "2.0",
                        "id": request.get("id"),
                        "result": {}
                    })),
                    "tools/list" => {
                        let tools = RepoMcpServer::get_tools();
                        Some(json!({
                            "jsonrpc": "2.0",
                            "id": request.get("id"),
                            "result": {
                                "tools": tools
                            }
                        }))
                    }
                    "tools/call" => {
                        if let Some(params) = request.get("params") {
                            let name = params.get("name").and_then(|v| v.as_str());
                            let args = params.get("arguments").cloned().unwrap_or(json!({}));

                            if let Some(name) = name {
                                match RepoMcpServer::execute_tool(name, args).await {
                                    Ok(result_value) => Some(json!({
                                        "jsonrpc": "2.0",
                                        "id": request.get("id"),
                                        "result": result_value
                                    })),
                                    Err(e) => Some(json!({
                                        "jsonrpc": "2.0",
                                        "id": request.get("id"),
                                        "result": {
                                            "content": [{
                                                "type": "text",
                                                "text": e.to_string()
                                            }],
                                            "isError": true
                                        }
                                    })),
                                }
                            } else {
                                Some(json!({
                                    "jsonrpc": "2.0",
                                    "id": request.get("id"),
                                    "error": {
                                        "code": -32602,
                                        "message": "Missing 'name' in params"
                                    }
                                }))
                            }
                        } else {
                            Some(json!({
                                "jsonrpc": "2.0",
                                "id": request.get("id"),
                                "error": {
                                    "code": -32602,
                                    "message": "Missing 'params'"
                                }
                            }))
                        }
                    }
                    // For unknown methods, reply only when this is a request (has id).
                    _ => {
                        if request.get("id").is_some() {
                            Some(json!({
                                "jsonrpc": "2.0",
                                "id": request.get("id"),
                                "error": {
                                    "code": -32601,
                                    "message": "Method not found"
                                }
                            }))
                        } else {
                            None
                        }
                    }
                };

                if let Some(response) = response {
                    if let Ok(data) = serde_json::to_string(&response) {
                        if tx
                            .send(Ok(Event::default().event("message").data(data)))
                            .is_err()
                        {
                            state.sessions.write().await.remove(&session_id);
                        }
                    }
                }
            } else {
                tracing::warn!("Received invalid JSON-RPC request: missing method");
                let error_response = json!({
                    "jsonrpc": "2.0",
                    "id": request.get("id"),
                    "error": {
                        "code": -32600,
                        "message": "Invalid Request: Method missing"
                    }
                });
                if let Ok(data) = serde_json::to_string(&error_response) {
                    if tx
                        .send(Ok(Event::default().event("message").data(data)))
                        .is_err()
                    {
                        state.sessions.write().await.remove(&session_id);
                    }
                }
            }
        });

        axum::http::StatusCode::ACCEPTED
    } else {
        axum::http::StatusCode::NOT_FOUND
    }
}
