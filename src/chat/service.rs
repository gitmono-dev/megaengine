use crate::gossip::message::{
    ChatAckMessage, EncryptedChatMessage, Envelope, GossipMessage, SignedMessage,
};
use crate::node::node::Node;
use crate::node::node_id::NodeId;
use crate::storage::chat_message::MessageStatus;
use crate::transport::quic::ConnectionManager;
use crate::util::timestamp_now;
use anyhow::{anyhow, Result};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

const TTL: u8 = 16;

pub async fn start_chat_sender_task(
    manager: Arc<Mutex<ConnectionManager>>,
    my_node: Node,
) -> Result<()> {
    loop {
        if let Err(e) = process_pending_messages(manager.clone(), my_node.clone()).await {
            tracing::error!("Failed to process pending messages: {}", e);
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
    }
}

async fn process_pending_messages(
    manager: Arc<Mutex<ConnectionManager>>,
    my_node: Node,
) -> Result<()> {
    // 1. Find all messages with status 'Sending'
    let db = crate::storage::get_db_conn().await?;
    let pending_msgs = crate::storage::chat_message::Entity::find()
        .filter(crate::storage::chat_message::Column::Status.eq(MessageStatus::Sending))
        .all(&db)
        .await?;
    
    for msg in pending_msgs {
        tracing::info!("Processing pending message: {}", msg.id);

        let receiver_node_id = match NodeId::from_string(&msg.to) {
            Ok(id) => id,
            Err(_) => {
                tracing::error!("Invalid receiver node id: {}, marking failed", msg.to);
                crate::storage::chat_message::update_message_status(&msg.id, MessageStatus::Failed).await?;
                continue;
            }
        };

        match try_send_pending_msg(manager.clone(), my_node.clone(), receiver_node_id, msg.content.clone(), msg.id.clone()).await {
             Ok(_) => {
                 crate::storage::chat_message::update_message_status(&msg.id, MessageStatus::Sent).await?;
                 tracing::info!("Message {} sent successfully", msg.id);
             },
             Err(e) => {
                 tracing::error!("Failed to send message {}: {}", msg.id, e);
                 // We can keep it as 'Sending' to retry later, or make a 'Failed' logic
                 // For now, retry indefinitely
             }
        }
    }
    Ok(())
}

async fn try_send_pending_msg(
    manager: Arc<Mutex<ConnectionManager>>,
    my_node: Node,
    receiver_node_id: NodeId,
    content: String,
    msg_id: String,
) -> Result<()> {
    // 1. Get Receiver Public Key
    let receiver_keypair = receiver_node_id.to_keypair()
        .map_err(|_| anyhow!("Could not decode receiver NodeId (did:key)"))?;
    let receiver_pk = receiver_keypair.verifying_key;

    // 2. Encrypt
    let my_keypair = &my_node.keypair;
    let encrypted_bytes = my_keypair.encrypt_to_node(&receiver_pk, content.as_bytes())?;

    // 3. Construct Message
    let encrypted_chat = EncryptedChatMessage {
        sender_id: my_node.node_id().clone(),
        receiver_id: receiver_node_id.clone(),
        msg_id: msg_id.clone(),
        ciphertext: encrypted_bytes,
    };
    
    let message = GossipMessage::Chat(encrypted_chat);
    
    // 4. Sign & Broadcast/Send
    let mut signed_msg = SignedMessage {
        node_id: my_node.node_id().clone(),
        message,
        timestamp: timestamp_now(),
        signature: "".to_string(),
    };
    let self_hash = signed_msg.self_hash();
    let sign = my_node.sign_message(self_hash.as_slice())?;
    signed_msg.signature = hex::encode(sign);

    let envelope = Envelope {
        payload: signed_msg,
        ttl: TTL,
    };
    let data = serde_json::to_vec(&envelope)?;

    let mgr = manager.lock().await;

    // Try to find if we are connected or have a known route?
    // Gossip usually just floods peers if not knowing better.
    // If we have direct connection or routing table, use it.
    // For now: broadcast to all connected peers.

    let peers = mgr.list_peers().await;
    if peers.is_empty() {
        return Err(anyhow!("No peers connected to send message"));
    }

    if peers.contains(&receiver_node_id) {
        // Direct send to receiver; propagate any error to the caller.
        mgr.send_gossip_message(receiver_node_id.clone(), data.clone())
            .await
            .map_err(|e| anyhow!("Failed to send gossip message to receiver {}: {}", receiver_node_id, e))?;
    } else {
        // Broadcast to all peers; require at least one successful send.
        let mut at_least_one_success = false;
        let mut last_err: Option<anyhow::Error> = None;

        for peer in peers {
            match mgr.send_gossip_message(peer.clone(), data.clone()).await {
                Ok(()) => {
                    at_least_one_success = true;
                }
                Err(e) => {
                    last_err = Some(anyhow!("Failed to send gossip message to peer {}: {}", peer, e));
                }
            }
        }

        if !at_least_one_success {
            // If all sends failed, return the last error (or a generic one if none captured).
            if let Some(err) = last_err {
                return Err(err);
            } else {
                return Err(anyhow!("Failed to send gossip message to any peer (unknown error)"));
            }
        }
    }

    Ok(())
}

pub async fn send_chat_message(
    _manager: Arc<Mutex<ConnectionManager>>,
    my_node: Node,
    receiver_node_id: NodeId,
    content: String,
) -> Result<()> {
    // Backward compatibility or direct call wrapper
    // Now just saves to DB and calls try_send immediately for responsiveness, or let scheduler handle it?
    // Let's make it just save to DB.

    let msg_id = Uuid::new_v4().to_string();

    crate::storage::chat_message::save_message(
        msg_id.clone(),
        my_node.node_id().to_string(),
        receiver_node_id.to_string(),
        content.clone(),
        timestamp_now(),
        MessageStatus::Sending,
    )
    .await?;
    
    // Maybe trigger one round of processing immediately?
    // For now rely on background task.
    Ok(())
}

pub async fn process_incoming_chat(
    msg: EncryptedChatMessage,
    manager: Arc<Mutex<ConnectionManager>>,
    my_node: Node,
) -> Result<()> {
    // 1. Check if it's for me
    if msg.receiver_id != *my_node.node_id() {
        tracing::info!("Message not for me, forwarding to {}", msg.receiver_id);

        // Construct the payload to forward
        let gossip_msg = GossipMessage::Chat(msg.clone());

        let mut signed_msg = SignedMessage {
            node_id: my_node.node_id().clone(),
            message: gossip_msg,
            timestamp: timestamp_now(),
            signature: "".to_string(),
        };
        let self_hash = signed_msg.self_hash();
        let sign = my_node.sign_message(self_hash.as_slice())?;
        signed_msg.signature = hex::encode(sign);

        let envelope = Envelope {
            payload: signed_msg,
            ttl: TTL,
        };
        let data = serde_json::to_vec(&envelope)?;

        let mgr = manager.lock().await;
        let peers = mgr.list_peers().await;

        if peers.contains(&msg.receiver_id) {
            tracing::info!("Found target {} in neighbors, sending directly", msg.receiver_id);
            let _ = mgr.send_gossip_message(msg.receiver_id.clone(), data).await;
        } else {
            tracing::info!("Target {} not in neighbors, broadcasting", msg.receiver_id);
            for peer in peers {
                if peer != msg.sender_id {
                    let _ = mgr.send_gossip_message(peer.clone(), data.clone()).await;
                }
            }
        }

        return Ok(());
    }

    // 2. Decrypt
    let my_keypair = &my_node.keypair;
    let plaintext_bytes = my_keypair.decrypt_message(&msg.ciphertext)?;
    let content = String::from_utf8(plaintext_bytes)?;
    
    tracing::info!("Received Chat from {}: {}", msg.sender_id.0, content);

    // 3. Store
    let db = crate::storage::get_db_conn().await.unwrap();
    if (crate::storage::chat_message::Entity::find_by_id(msg.msg_id.clone()).one(&db).await?).is_none() {
        crate::storage::chat_message::save_message(
            msg.msg_id.clone(),
            msg.sender_id.to_string(),
            my_node.node_id().to_string(),
            content,
            timestamp_now(),
            MessageStatus::Delivered,
        )
        .await?;
    }

    // 4. Send ACK
    let ack_msg = ChatAckMessage {
        sender_id: my_node.node_id().clone(),
        target_id: msg.sender_id.clone(),
        msg_id: msg.msg_id.clone(),
        timestamp: timestamp_now(),
        signature: "".to_string(),
    };
    
    let ack_sig = my_node.sign_message(msg.msg_id.as_bytes())?;
    let ack_msg = ChatAckMessage {
        signature: hex::encode(ack_sig),
        ..ack_msg
    };

    let gossip_msg = GossipMessage::ChatAck(ack_msg);
    
    let mut signed_ack = SignedMessage {
        node_id: my_node.node_id().clone(),
        message: gossip_msg,
        timestamp: timestamp_now(),
        signature: "".to_string(),
    };
    let self_hash = signed_ack.self_hash();
    let sign = my_node.sign_message(self_hash.as_slice())?;
    signed_ack.signature = hex::encode(sign);
    
    let envelope = Envelope {
        payload: signed_ack,
        ttl: TTL,
    };
    let data = serde_json::to_vec(&envelope)?;
    
    let mgr = manager.lock().await;
    let peers = mgr.list_peers().await;
    for peer in peers {
         let _ = mgr.send_gossip_message(peer.clone(), data.clone()).await;
    }
    
    Ok(())
}

pub async fn process_ack(
    ack: ChatAckMessage,
    manager: Arc<Mutex<ConnectionManager>>,
    my_node: Node,
) -> Result<()> {
    // 1. Check if it's for me
    if ack.target_id != *my_node.node_id() {
        tracing::info!("ACK not for me (for {}), forwarding", ack.target_id);
    
        // Construct the payload to forward
        let gossip_msg = GossipMessage::ChatAck(ack.clone());

        let mut signed_msg = SignedMessage {
            node_id: my_node.node_id().clone(),
            message: gossip_msg,
            timestamp: timestamp_now(),
            signature: "".to_string(),
        };
        let self_hash = signed_msg.self_hash();
        let sign = my_node.sign_message(self_hash.as_slice())?;
        signed_msg.signature = hex::encode(sign);

        let envelope = Envelope {
            payload: signed_msg,
            ttl: TTL,
        };
        let data = serde_json::to_vec(&envelope)?;

        let mgr = manager.lock().await;
        let peers = mgr.list_peers().await;

        if peers.contains(&ack.target_id) {
            tracing::info!("Found target {} in neighbors, sending ACK directly", ack.target_id);
            let _ = mgr.send_gossip_message(ack.target_id.clone(), data).await;
        } else {
            tracing::info!("Target {} not in neighbors, broadcasting ACK", ack.target_id);
            for peer in peers {
                if peer != ack.sender_id {
                    let _ = mgr.send_gossip_message(peer.clone(), data.clone()).await;
                }
            }
        }
        return Ok(());
    }

    tracing::info!("Received ACK for msg {}", ack.msg_id);
    
    crate::storage::chat_message::update_message_status(
        &ack.msg_id,
        MessageStatus::Delivered,
    )
    .await?;

    Ok(())
}

