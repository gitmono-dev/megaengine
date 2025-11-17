use crate::gossip::message::{GossipMessage, SignedMessage};
use crate::node::node::Node;
use crate::node::node_id::NodeId;
use crate::repo::repo_manager::RepoManager;
use crate::transport::quic::ConnectionManager;
use anyhow::Result;
use ed25519_dalek::Signature;
use hex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryInto;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};

const DEFAULT_TTL: u8 = 16;

/// 简单的 gossip 服务：接收来自 QUIC 的消息，去重、验签、处理并转发给邻居
#[allow(dead_code)]
pub struct GossipService {
    manager: Arc<Mutex<ConnectionManager>>,
    node: Node,
    repo_manager: Option<Arc<Mutex<RepoManager>>>,
    seen: Arc<Mutex<HashMap<String, Instant>>>,
}

impl GossipService {
    pub fn new(
        manager: Arc<Mutex<ConnectionManager>>,
        node: Node,
        repo_manager: Option<Arc<Mutex<RepoManager>>>,
    ) -> Self {
        Self {
            manager,
            node,
            repo_manager,
            seen: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Start the gossip service: register incoming channel and spawn handler + periodic broadcaster
    pub async fn start(self: Arc<Self>) -> Result<()> {
        let (tx, mut rx) = mpsc::channel::<(NodeId, Vec<u8>)>(256);

        // register incoming sender with connection manager
        {
            let mgr = self.manager.lock().await;
            mgr.register_incoming_sender(tx).await;
        }

        // clone for handler task
        let s = Arc::clone(&self);
        tokio::spawn(async move {
            while let Some((from, data)) = rx.recv().await {
                let _ = s.handle_incoming(from, data).await;
            }
        });

        // periodic broadcaster: node announcement (and repo announcement if available)
        let s2 = Arc::clone(&self);
        tokio::spawn(async move {
            loop {
                #[derive(Serialize, Deserialize, Clone)]
                struct Envelope {
                    payload: SignedMessage,
                    ttl: u8,
                }

                // 1. 发送 NodeAnnouncement
                if let Ok(signed) = SignedMessage::new_node_sign_message(s2.node.clone()) {
                    let env = Envelope {
                        payload: signed,
                        ttl: DEFAULT_TTL,
                    };
                    let data = serde_json::to_vec(&env).unwrap_or_default();
                    let mgr = s2.manager.lock().await;
                    let peers = mgr.list_peers().await;
                    for peer in peers {
                        let _ = mgr.send_message(peer.clone(), data.clone()).await;
                    }
                }

                // 2. 发送 RepoAnnouncement（从本地 storage 加载 repo 列表）
                if let Ok(repos) = crate::storage::repo_model::list_repos().await {
                    let repo_ids: Vec<_> = repos
                        .iter()
                        .filter_map(|r| {
                            crate::repo::repo_id::RepoId::parse_from_str(&r.repo_id).ok()
                        })
                        .collect();
                    if !repo_ids.is_empty() {
                        if let Ok(signed) =
                            SignedMessage::new_repo_sign_message(repo_ids, s2.node.clone())
                        {
                            let env = Envelope {
                                payload: signed,
                                ttl: DEFAULT_TTL,
                            };
                            let data = serde_json::to_vec(&env).unwrap_or_default();
                            let mgr = s2.manager.lock().await;
                            let peers = mgr.list_peers().await;
                            for peer in peers {
                                let _ = mgr.send_message(peer.clone(), data.clone()).await;
                            }
                        }
                    }
                }

                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        });

        // spawn a cleanup task for seen map
        let seen = Arc::clone(&self.seen);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let mut guard = seen.lock().await;
                let now = Instant::now();
                guard.retain(|_, &mut v| v + Duration::from_secs(300) > now);
            }
        });

        Ok(())
    }

    async fn handle_incoming(&self, from: NodeId, data: Vec<u8>) -> Result<()> {
        // Try parse as Envelope (with ttl). If not, fall back to raw SignedMessage.
        #[derive(Serialize, Deserialize, Clone)]
        struct Envelope {
            payload: SignedMessage,
            ttl: u8,
        }

        let (signed, mut ttl) = if let Ok(env) = serde_json::from_slice::<Envelope>(&data) {
            (env.payload, env.ttl)
        } else if let Ok(s) = serde_json::from_slice::<SignedMessage>(&data) {
            (s, DEFAULT_TTL)
        } else {
            return Ok(());
        };

        let id = hex::encode(signed.self_hash());

        // dedup
        {
            let mut seen = self.seen.lock().await;
            if seen.contains_key(&id) {
                return Ok(());
            }
            seen.insert(id.clone(), Instant::now());
        }

        // verify signature using sender's NodeId -> verifying key
        if let Ok(kp) = signed.node_id.to_keypair() {
            let sig_bytes = hex::decode(&signed.signature).unwrap_or_default();
            let arr: [u8; 64] = match sig_bytes.as_slice().try_into() {
                Ok(a) => a,
                Err(_) => return Ok(()),
            };
            let sig = Signature::from_bytes(&arr);
            if !kp.verify(&signed.self_hash(), &sig) {
                tracing::warn!(
                    "signature verification failed for message from {}",
                    signed.node_id
                );
                return Ok(());
            }
        }

        // process message (borrow the inner message to avoid moving)
        match &signed.message {
            GossipMessage::NodeAnnouncement(na) => {
                tracing::info!(
                    "Gossip: NodeAnnouncement from {} (alias: {})",
                    na.node_id,
                    na.alias
                );
                // TODO: update NodeManager or Node routing table
            }
            GossipMessage::RepoAnnouncement(ra) => {
                tracing::info!(
                    "Gossip: RepoAnnouncement from {} with {} repos: {:?}",
                    ra.node_id,
                    ra.repos.len(),
                    ra.repos.iter().map(|r| r.as_str()).collect::<Vec<_>>()
                );
                // 可选：记录该节点拥有的 repo 信息到本地数据库（用于搜索/发现）
                // 这里可以保存到一个 peer_repos 表用于跟踪哪个节点有哪些 repo
            }
        }

        // forward if ttl > 0
        if ttl > 0 {
            ttl -= 1;
            #[derive(Serialize, Deserialize, Clone)]
            struct Envelope2 {
                payload: SignedMessage,
                ttl: u8,
            }
            let fwd = Envelope2 {
                payload: signed.clone(),
                ttl,
            };
            let data = serde_json::to_vec(&fwd).unwrap_or_default();
            let mgr = self.manager.lock().await;
            let peers = mgr.list_peers().await;
            for peer in peers {
                if peer == from {
                    continue;
                }
                let _ = mgr.send_message(peer.clone(), data.clone()).await;
            }
        }

        Ok(())
    }
}
