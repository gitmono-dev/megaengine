use crate::node::{
    node::{Node, NodeRouting},
    node_id::NodeId,
};
use std::collections::HashMap;

#[derive(Default)]
pub struct NodeManager {
    pub nodes: HashMap<NodeId, NodeRouting>,
}

impl NodeManager {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    pub fn insert_node(&mut self, node: &Node) {
        let routing = NodeRouting::new(node.node_id().clone(), node.addresses().to_vec());
        self.nodes.insert(node.node_id().clone(), routing);
    }

    pub fn mark_alive(&mut self, node_id: &NodeId) {
        if let Some(n) = self.nodes.get_mut(node_id) {
            n.refresh();
        }
    }

    pub fn cleanup_expired(&mut self) {
        self.nodes.retain(|_, v| !v.expired());
    }

    pub fn get_node(&self, node_id: &NodeId) -> Option<&NodeRouting> {
        self.nodes.get(node_id)
    }

    pub fn routing_print(&self) {
        println!("Node routing table ({} entries):", self.nodes.len());
        for (id, info) in &self.nodes {
            println!("  {:?} -> {:?}", id, info.addresses);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::keypair::{self, KeyPair};
    use crate::node::node::NodeType;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn create_sample_node() -> Node {
        let keypair = &KeyPair::generate().unwrap();
        let node_id = NodeId::from_keypair(keypair);
        let alias = "Test Node";
        let addresses = vec![SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            8080,
        )];
        let node_type = NodeType::Normal;

        Node::new(node_id, alias, addresses, node_type, keypair.clone())
    }

    #[test]
    fn test_node_manager_insert_node() {
        let mut manager = NodeManager::new();
        let node = create_sample_node();

        // Insert the node
        manager.insert_node(&node);

        // Assert that the node is in the manager
        assert_eq!(manager.nodes.len(), 1);
        let node_routing = manager.get_node(&node.node_id());
        assert!(node_routing.is_some());
        assert_eq!(node_routing.unwrap().node_id, *node.node_id());
    }

    #[test]
    fn test_node_manager_mark_alive() {
        let mut manager = NodeManager::new();
        let node = create_sample_node();

        // Insert the node
        manager.insert_node(&node);

        // Get the initial last_seen time
        let initial_last_seen = manager.get_node(&node.node_id()).unwrap().last_seen;

        // Mark the node as alive (refresh)
        manager.mark_alive(&node.node_id());

        // Assert that the last_seen time was refreshed
        let refreshed_last_seen = manager.get_node(&node.node_id()).unwrap().last_seen;
        assert_ne!(initial_last_seen, refreshed_last_seen);
    }

    #[test]
    fn test_node_manager_cleanup_expired() {
        let mut manager = NodeManager::new();
        let node = create_sample_node();

        manager.insert_node(&node);
        assert_eq!(manager.nodes.len(), 1);

        manager.nodes.get_mut(&node.node_id()).unwrap().ttl = std::time::Duration::from_secs(1);
        std::thread::sleep(std::time::Duration::from_secs(2));
        manager.cleanup_expired();
        assert_eq!(manager.nodes.len(), 0);
    }

    #[test]
    fn test_node_manager_routing_print() {
        let mut manager = NodeManager::new();
        let node1 = create_sample_node();
        let node2 = create_sample_node();

        manager.insert_node(&node1);
        manager.insert_node(&node2);

        let _ = std::panic::catch_unwind(|| {
            manager.routing_print();
        });
    }
}
