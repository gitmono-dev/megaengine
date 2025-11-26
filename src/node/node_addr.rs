use crate::node::node_id::NodeId;
use anyhow::{anyhow, Result};
use std::net::SocketAddr;

/// Represents a node address in the format: peer_id@address
/// Example: did:key:z2DeZG8TuHkTvrJ7jijysNsQTpTiu9tRQkxcPmmem1tHvVP@127.0.0.1:9000
#[derive(Debug, Clone)]
pub struct NodeAddr {
    pub peer_id: NodeId,
    pub address: SocketAddr,
}

impl NodeAddr {
    /// Parse node address from string format: "peer_id@address"
    pub fn parse(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split('@').collect();
        if parts.len() != 2 {
            return Err(anyhow!(
                "Invalid node address format. Expected 'peer_id@address', got '{}'",
                s
            ));
        }

        let peer_id_str = parts[0];
        let address_str = parts[1];

        let peer_id = peer_id_str
            .parse::<NodeId>()
            .map_err(|_| anyhow!("Invalid peer_id: {}", peer_id_str))?;

        let address = address_str
            .parse::<SocketAddr>()
            .map_err(|_| anyhow!("Invalid address: {}", address_str))?;

        Ok(NodeAddr { peer_id, address })
    }

    /// Create  node address from peer_id and address
    pub fn new(peer_id: NodeId, address: SocketAddr) -> Self {
        NodeAddr { peer_id, address }
    }
}

impl std::fmt::Display for NodeAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}", self.peer_id, self.address)
    }
}

#[cfg(test)]
mod tests {}
