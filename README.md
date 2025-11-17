# MegaEngine - P2P Git Network

MegaEngine is a distributed peer-to-peer (P2P) network for Git repositories. It enables nodes to discover, announce, and synchronize Git repositories across a decentralized network using the gossip protocol over QUIC transport.

## 🎯 Features

- **Decentralized Node Discovery**: Nodes automatically discover each other and exchange node information via gossip protocol
- **Repository Synchronization**: Nodes announce and sync repository inventory across the network
- **QUIC Transport**: Uses QUIC protocol for reliable, low-latency peer-to-peer communication
- **Gossip Protocol**: Implements epidemic message propagation with TTL and deduplication
- **Cryptographic Identity**: Each node has a unique EdDSA-based identity (`did:key` format)
- **SQLite Persistence**: Stores repositories and node information persistently
- **CLI Interface**: Easy-to-use command-line tool for managing nodes and repositories

## 📦 Architecture

```
┌─────────────────────────────────────┐
│         CLI Interface               │
│  (node start, repo add, auth init)  │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│      Node / Repository Manager      │
│   (NodeManager, RepoManager)        │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│    Gossip Protocol Service          │
│ (message relay, dedup, TTL)         │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│    QUIC Connection Manager          │
│  (peer connections, message send)   │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│   SQLite Storage / Sea-ORM         │
│  (repos, nodes persistence)         │
└─────────────────────────────────────┘
```

## 🔧 Build & Setup

### Prerequisites

- Rust 1.70+ (2021 edition)
- Git
- OpenSSL development libraries (for TLS)

### Build

```bash
cargo build --release
```

### Configure Environment

Set the root directory for MegaEngine data (default: `~/.megaengine`):

```bash
export MEGAENGINE_ROOT=/path/to/megaengine-data
```

## 🚀 Usage

### 1. Initialize Keypair

Generate a new cryptographic keypair (EdDSA):

```bash
cargo run -- auth init
```

Output:
```
Keypair saved to <root>/.megaengine/keypair.json
```

### 2. Start a Node

Start a MegaEngine node that listens on a QUIC endpoint:

```bash
cargo run -- node start \
  --alias my-node \
  --addr 0.0.0.0:9000 \
  --cert-path cert
```

The node will:
- Initialize QUIC server on the specified address
- Start gossip protocol for peer discovery
- Periodically announce node and repository information
- Listen indefinitely until Ctrl+C

### 3. Get Node ID

Display the node ID (based on your keypair):

```bash
cargo run -- node id
```

Output:
```
did:key:z2DXbAovGq5vNKpXVFyrhVLppMdUCmV1hCNjbUydLMEWasE
```

### 4. Register a Repository

Add a local Git repository to the network:

```bash
cargo run -- repo add \
  --path /path/to/git/repo \
  --description "My awesome repository"
```

The repo ID is automatically generated from the Git root commit hash and the node's public key.

## 🧪 Testing

Run the integration test for three-node gossip propagation:

```bash
cargo test --test gossip_three_nodes -- --nocapture --test-threads=1
```

This test:
1. Starts three nodes with QUIC servers
2. Connects them in a chain: node1 ↔ node2 ↔ node3
3. Initiates gossip protocol on all nodes
4. Sends NodeAnnouncement and RepoAnnouncement messages
5. Verifies message propagation with TTL and deduplication

Example output:
```
Gossip: NodeAnnouncement from did:key:z2DZe... (alias: node1)
Gossip: RepoAnnouncement from did:key:z2DW... with 1 repos: ["did:repo:zW1i..."]
```

## 📁 Project Structure

```
src/
├── main.rs                  # CLI entry point
├── lib.rs                   # Library root
├── identity/
│   └── keypair.rs          # EdDSA keypair generation and signing
├── node/
│   ├── node.rs             # Node runtime with QUIC manager
│   ├── node_id.rs          # Node identifier (did:key format)
│   ├── node_manager.rs     # Node routing and lifecycle
│   └── node.rs
├── repo/
│   ├── repo.rs             # Repository metadata and refs
│   ├── repo_id.rs          # Repository identifier (did:repo format)
│   └── repo_manager.rs     # Repository persistence and query
├── storage/
│   ├── mod.rs              # Database initialization and connection
│   ├── repo_model.rs       # Sea-ORM models and CRUD for repos
│   └── node_model.rs       # Sea-ORM models and CRUD for nodes
├── transport/
│   ├── quic.rs             # QUIC connection manager
│   ├── config.rs           # QUIC configuration
│   └── cert.rs             # TLS certificate generation
├── gossip/
│   ├── mod.rs              # Gossip service exports
│   ├── message.rs          # Gossip message types and signing
│   └── service.rs          # Gossip protocol implementation
├── git/
│   └── mod.rs              # Git repository utilities
└── util/
    └── mod.rs              # Timestamp and utility functions

tests/
└── gossip_three_nodes.rs    # Integration test for gossip relay
```

## 🔐 Data Formats

### Node ID (did:key)

```
did:key:z2DSQWVWxVg2Dq8qvq7TqJG75gY2hh9cT6RkzzgYpf7YptF
       ↑  ↑    ↑
       |  |    Ed25519 public key (base58 encoded)
       |  Multibase encoding
       DID scheme
```

### Repository ID (did:repo)

```
did:repo:zW1iF5iwCChifAcjZUrDbwD9o8LS76kFsz6bTZFEJhEqVCU
        ↑      ↑
        |      SHA3-256(root_commit + creator_pubkey)
        Multibase encoding
```

## 📊 Gossip Protocol

- **Message Types**:
  - `NodeAnnouncement`: Advertises node metadata (alias, addresses, type)
  - `RepoAnnouncement`: Lists repositories owned by a node

- **TTL (Time-to-Live)**: Default 16 hops, decremented on each relay
- **Deduplication**: Tracks seen message hashes in a 5-minute sliding window
- **Broadcast Interval**: 10 seconds

## 💾 Storage

Data is persisted in SQLite at `$MEGAENGINE_ROOT/megaengine.db`:

### Tables

- **repos**: Repository metadata (id, name, creator, description, path, refs, timestamps)
- **nodes**: Node information (id, alias, addresses, node_type, version, timestamps)

## 🔧 Configuration

### Environment Variables

- `MEGAENGINE_ROOT`: Root directory for data storage (default: `~/.megaengine`)
- `RUST_LOG`: Logging level (e.g., `megaengine=debug`)

### Default Ports

- QUIC Server: `0.0.0.0:9000` (configurable via `--addr`)


