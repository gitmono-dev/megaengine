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


## Example: Two-Node Network with Repository Synchronization

This example demonstrates how to set up a two-node network where the first node adds a repository, and the second node automatically synchronizes and clones it.

### Prerequisites

- Create a test Git repository (or use an existing one):
  ```bash
  mkdir -p E:\git_test\tiny
  cd E:\git_test\tiny
  git init
  # Add some content
  git add .
  git commit -m "Initial commit"
  ```

### Step 1: Initialize Keypairs for Both Nodes

**Terminal 1** - Initialize the first node's keypair:
```bash
cargo run -- auth init
```

**Terminal 2** - Initialize the second node's keypair with a custom root directory:
```bash
cargo run -- --root ~/.megaengine2 auth init
```

Output will show the keypair location and the DID key for each node.

### Step 2: Start Both Nodes

**Terminal 1** - Start the first node (node1):
```bash
cargo run -- node start --alias node1 --addr 127.0.0.1:9000 --cert-path cert
```

Keep this terminal running.

**Terminal 2** - Start the second node (node2) with node1 as bootstrap node:
```bash
cargo run -- --root ~/.megaengine2 node start --alias node2 --cert-path cert --bootstrap-node did:key:z2DUYGZos3YrXrD4pQ9aAku2g7btumKcfTiMSyBC8btqFDJ@127.0.0.1:9000 --addr 127.0.0.1:9001
```

Keep this terminal running as well.

**Note**: Replace `did:key:z2DUYGZos3YrXrD4pQ9aAku2g7btumKcfTiMSyBC8btqFDJ` with the actual DID key from the first node's auth init output.

### Step 3: Add Repository to Node1

**Terminal 3** - Add a repository on node1:
```bash
cargo run -- repo add --path /path/to/git_test/tiny --description "Tiny"
```

The output will display the repo ID. Save this ID for later use.

### Step 4: Node2 Automatically Synchronizes

The second node will automatically:
1. Discover the repository announcement via gossip protocol
2. Periodically request the bundle from node1 (every 60 seconds by default)
3. Download the bundle file
4. Store it locally

Monitor the output from Terminal 2 to see the synchronization progress.

### Step 5: Query Repository on Node2

**Terminal 3** - List repositories on node2:
```bash
cargo run -- --root ~/.megaengine2 repo list
```

You should see the "Tiny" repository announced by node1.

### Step 6: Clone Repository from Node2

**Terminal 3** - Clone the repository on node2:
```bash
cargo run -- --root ~/.megaengine2 repo clone --repo-id <repo_id> --output ./tiny
```

Replace `<repo_id>` with the ID from Step 3.

The cloned repository will be available at `./tiny` on node2.

### Step 7: Repository Update Synchronization

When the repository creator (node1) pushes new commits, node2 will automatically synchronize them.

**Terminal 3** - Make a change in the original repository on node1:
```bash
cd E:\git_test\tiny
# Add or modify some files
echo "Updated content" >> README.md
git add README.md
git commit -m "Update repository"
```

**Terminal 3** - Update the repository bundle on node1:
```bash
cargo run -- repo list
```

You'll see the status indicator changes to `⚠️  HAS UPDATES`, showing new commits are available.

**Terminal 3** - Node2 will automatically discover and download the updated bundle

Monitor Terminal 2 output - you should see automatic bundle sync activity. The background task runs every 60 seconds and will:
1. Detect the repository update announcement via gossip protocol
2. Request the updated bundle from node1
3. Download and store the new bundle

**Terminal 3** - Check repository status on node2:
```bash
cargo run -- --root ~/.megaengine2 repo list
```

You should see the repository status has changed, indicating updates are available.

**Terminal 3** - Pull the latest updates to the cloned repository on node2:
```bash
cargo run -- --root ~/.megaengine2 repo pull --repo-id <repo_id>
```

Replace `<repo_id>` with the repository ID from Step 3.

The cloned repository at `./tiny` will be updated with the latest commits from the bundle.



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


