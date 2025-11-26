# MegaEngine - P2P Git Network

MegaEngine is a distributed peer-to-peer (P2P) network for Git repositories. It enables nodes to discover, announce, and synchronize Git repositories across a decentralized network using the gossip protocol over QUIC transport.

## 🎯 Features

- **Decentralized Node Discovery**: Nodes automatically discover each other and exchange node information via gossip protocol
- **Repository Synchronization**: Nodes announce and sync repository inventory across the network
- **Bundle Transfer**: P2P transfer of Git bundle files between nodes with integrity verification
- **Automatic Bundle Sync**: Periodic background task that automatically downloads bundles for external repositories
- **Repository Cloning**: Clone repositories from bundles using the `repo clone` command
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
- Git (for git operations and bundle/tar packing)
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
  --description "My repository"
```

The repo ID is automatically generated from the Git root commit hash and the node's public key.

### 5. Clone a Repository

Clone a repository from bundle using its repo ID:

```bash
cargo run -- repo clone \
  --repo-id <repo_id> \
  --output /path/to/clone
```

Requirements:
- Repository must exist in the database
- Bundle file must be downloaded (via automatic sync or transfer)


## 🧪 Testing



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

## 📦 Bundle Transfer Protocol

MegaEngine implements a multi-frame bundle transfer protocol for P2P repository synchronization:

### Message Types

- **Request**: Request a bundle for a repository from a peer
- **Start**: Initiates bundle transfer with metadata (file_name, total_size)
- **Chunk**: Transfers data in 64KB chunks
- **Done**: Signals transfer completion

### Workflow

1. **Discovery**: Node learns about external repository via gossip
2. **Request**: Background task periodically requests missing bundles from repo owner
3. **Generation**: Owner generates bundle from local repository
4. **Transfer**: Bundle is sent to requester in multiple frames
5. **Storage**: Received bundle is stored locally and marked in database
6. **Restoration**: User can clone repository from stored bundle

### Automatic Synchronization

- Runs every 60 seconds by default
- Checks for external repositories with empty bundle field
- Automatically requests missing bundles from repository owners

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


