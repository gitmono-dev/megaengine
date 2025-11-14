# 基于QUIC协议构建P2P的Git网络

核心：发现节点，发现仓库，共享仓库，复制仓库，开源协作

尽量与git本身无关，只需要一个仓库标识，就可以下载仓库，并验证，无论上传者是否在线

## 架构设计

- **Identity层：**每个节点有一对长期密钥（Ed25519）。签名用于消息 refs、repo、node messages 签名验证。
- **Transport层**：QUIC（使用 Rust 的 `quinn` 实现），或者Noise协议（Rust的`snow`实现，Noise协议只需要验证公钥就可以建立加密通信）。利用 QUIC 的 multiplexed streams 传输 git 数据，用 datagram 承载低价值的 gossip。
- **Gossip 层**：实现 RepoMessage/ NodeMessage/EventMessage （三类），负责节点表和仓库路由表构造与更新。
  - Messages会缓存一段时间，重复消息被丢弃
  -  **Repo Routing**：根据NodeMessage和RepoMessages创建仓库与节点的对应表。
    - 可以记录仓库的id，节点关系等
  - **Node Routing**：根据NodeMessage创建节点路由表
    - 可以记录节点的在线状态，优先级，地址等信息
- **Git层**：实现基于 Git 协议的对象传输，或支持 `git packfile` 的二进制流传输。
- **Relay层**：缓存RepoMessage/ RefsMessage / NodeMessage，缓存Repo仓库，推导Routing表。
- **应用层**：连接Relay，下载Relay的Routing表，实现分享仓库，下载仓库，更新仓库，开源协作等功能。

<img src="C:\Users\ASUS\AppData\Roaming\Typora\typora-user-images\image-20250922084407689.png" alt="image-20250922084407689" style="zoom:45%;" />

## 使用Gossip传播信息

1. 节点会把消息传播至邻居节点
2. 邻居节点收到消息会再次转发
3. 为避免无限传播，节点会丢弃重复收到的消息
4. 节点通常会缓存一定量的消息，用于判断消息是否已经收到

## 一、节点

### 节点id

`did:key:zQmW8QYFL8YRxq0QNqSLCVJvEoDuCgZQpL9FxT3p2ZbwU9o`

**NodeId 表示形式**：采用 **Multibase Base58-btc** 编码：

- 把 Ed25519 公钥原始字节前置 multicodec 标识（`0xED`，multicodec name: `ed25519-pub`）。
- 对这个字节串做 multibase(base58btc) 编码，并在前面加上 `did:key:` 。
- 示例最终形式： `did:key:z<base58str>` 

即`"did:key:z" + base58(multicodec_prefix || pubkey_bytes)`

节点id和公钥可以直接**互转**

### 节点

```
Node{
	NodeId,       //节点ID,与其公钥可以互相转化
	Alias,        //别名
	Vec<Address>, //节点地址,可以有多个
	NodeType,     //节点类型,normal/relay
	Version,      //版本u8
}
```

一般来说，节点需要缓存收到的节点消息一定时间

一方面是为了不重复发送消息，另一方面是为了大致了解网络节点的状态

### 节点发现

当一个新节点第一次加入网络时，需要「引导（bootstrap）节点（也可以称之为seed/relay节点）」：

1. 连接到Relay节点，接受Relay节点缓存的`Repo Routing Table`和`Node Routing Table`
2. 节点可以自己选择是否连接到其他Relay节点
3. 可以周期广播自己的`NodeMessage`以便其他节点更新自己的在线状态

## 二、仓库

### P2P仓库

仓库id生成：

`did:repo:zXXXXXXXXXX`

```
Repo{
	RepoId,
	GitRootCommit,
	Refs,
	P2PDescription,
	Path,   //git仓库地址
}

//git仓库关联
//git的首次提交记录 + 创建者公钥 -> p2p仓库id
RepoId = Multibase(Multihash(
    GitRootCommit || CreatorPublicKey  //一次性使用
))

P2PDescription{
	"creator":"NodeId",
	"name":"aaa",
	"description":"bbb",
	"timestamp":123
}
```

### 本地仓库

需要建立本地仓库和P2P仓库的对应关系

RepoId -> .git的文件路径

这个关系存在本地

### P2P仓库地址

1.`git+p2p://{RepoId}`

这个地址算是短地址，但是由于同一个仓库，可能有多个拷贝。

默认从Relay节点克隆仓库，默认选择长期在线节点，Relay需要跟原始地址保持一致数据

2.`git+p2p://{RepoId}/peer/{NodeId}`

从某个节点下载某个仓库，但是一般节点可能无法直连，可以特指某个fork仓库

3.

`mega://{RepoId}/peer/{NodeId}/refs/{RefName}`

`mega://{RepoId}/peer/{NodeId}/path/{FilePath}`

`mega://{RepoId}/peer/{NodeId}/raw/{GitHash}`

找某个节点的Repo的文件路径 ，或者历史，或者提交信息等

### MonoRepo的情况

P2P设计是基于通用的Git来设计，还是根据Mega的Mono来设计？

几个问题？

1. 是否全网共用一个MonoRepo树？不是，每个client有自己的mono，其实跟本地git一样
2. 还有RepoId的概念吗，是不是用地址就可以？有RepoId，也有path
3. 多个人提交同一个地址，会冲突吗？不会

回答：

这个其实不用太考虑，看git用的哪个服务就行，如果是Mega，让Mega自己处理，如果是普通的git服务，让git服务自己处理。

## 三、消息Gossip Messages

### 节点消息NodeMessage

```
NodeMessage{
	NodeId,       //节点ID,与其公钥可以互相转化
	Alias,        //别名
	Vec<Address>, //节点地址,可以有多个
	NodeType,     //节点类型,normal/relay
	Version,      //版本u8
}
```

### 仓库消息RepoMessage

```
RepoMessage{
	NodeId,
	RepoId,
}
```

### 协作消息ActivityMessage 

```
ActivityMessage{
	NodeId,
	RepoId,
	Activity,
}
```

把消息包起来，并签名

```
Message{
	NodeId,
	Sign,
	Enum<NodeMessage,RepoMessage,ActivityMessage>
}
```



## 四、路由表

1. 将收到的RepoMessage和NodeMessage转化为：

```
struct NodeRouting{
  node_id: NodeId,
  addresses: Vec<Address>,
  last_seen: SystemTime,
  ttl: Duration,          //超过 TTL（比如 24 小时）未刷新则删除
  score: f32              // 用于优先级
}

//Map存下Repo和node的对应关系
RepoRouting: Map<RepoId, Vec<NodeRouting>>
```

Client是否需要这个路由表？

其实还有个功能，就是 本地 查询仓库和节点，我就可以本地直接查，就不用再去问relay了

## 五、传输层协议

### 如果是QUIC连接

#### 需要证书管理

Ca？Relay？如何管理根证书？

#### Multiplexed Streams

可靠传输机制，用于传输git pack流

#### Datagram 

不可靠传播机制，用于Gossip广播

#### 连接保持

Relay和普通节点需要 将NodeId 与 QUIC connection 绑定，方便广播和nat穿透

### 如果是Noise

TLS协议过于笨重，证书管理过程需要自己处理

`Noise`协议基于DH 算法，创建会话密钥，是基于TCP的可靠传输

radicle，libp2p（可选）用的也是Noise

### 如果公钥已知的情况下是否可以建立连接

可以自定义验证流程吗，因为双方公钥可以看作已知

## 六、Git数据传输

### 模仿git fetch

1. 客户端连接Relay获得`NodeRouting`、`RepoRouting`和`Repo`的数据
2. 客户端请求 Repo加Refs，默认从活跃度高的Relay请求
3. Relay 通过git打包 package 传输到client
4. 校验package并暂存本地，等待后端自己处理

## 七、NAT穿透

目前**不直接提供**NAT穿透功能，Message由Relay进行转发

仓库复制，如果是内网节点，需要等Relay先复制，才能给其他节点复制

如果是公网节点，可以直接提供服务，但不需要像Relay一样缓存大量的仓库

## 八、仓库一致性

如果仓库被多次复制，和提交，仓库可能会分叉

类似 **Radicle 的设计**：`refs/nodes/<node_id>/heads`，**每个节点独立维护自己的 refs，不覆盖别人，最终由协作策略patch协作收敛。**

如果Relay，收到同一个RepoId的不同refs，都保存，但是需要标明哪些是仓库代表`delegates`的refs。

思考：

开源协作场景下，PR怎么提交，仓库怎么更新

Radicle有，去看一下

回答：

Radicle使用的patch，类似pr，需要提交patch协作请求，由`delegate`下载到本地分支，合并到自己的分支并上传

## 九、开源协作

通过ActivityMessage去实现，全网广播

```
ActivityMessage{
	NodeId,
	RepoId,
	Activity,
}
```

### 协作对象类型

| 类型名 | 含义 | github类比 |
| ------ | ---- | ---------- |
| Issue  | 问题 | issue      |
| Patch  | 补丁 | PR         |

...

### 协作对象事件

```
Activity {
    id: Hash,                // 事件哈希
    parent: Hash,            // 上一个事件id,根是repoId
    type: Type               // Issue/Patch/Comment
    payload: Payload,        // 事件内容
    author: PeerId,          // 签名者
    signature: Signature,    // Ed25519 签名
    timestamp: u64,
}
```

### 协作对象负载

#### Issue

```
enum IssueEventPayload {
    Create { title: String, body: String },
    Comment { body: String },
    Close,
    Reopen,
    Edit { new_body: String },
}
```

#### Patch

```
enum PatchEventPayload {
    Create { base: CommitHash, head: CommitHash, title: String },
    Review { decision: ReviewDecision },
    Merge,
    Close,
}
```

### 对象存储

目前只需要存储Event即可

### 分叉与合并

冲突自由复制数据类型（Conflict-free Replicated Data Type, CRDT），在网络延迟和离线的情况下，保证最终结果一致性

![image-20251011163655788](C:\Users\ASUS\AppData\Roaming\Typora\typora-user-images\image-20251011163655788.png)



在去中心化环境中，不同节点（Peers）对同一个 Event的操作可能是**异步传播**的，因此：

当节点收到另一个分支时，它不会直接覆盖，而是：

1. 验证操作签名；
2. 将该操作插入本地 DAG；
3. 调用合并逻辑来生成一个统一的 view（视图）。

### Patch(PR)的流程

#### 本地分支提交

```
git checkout -b fix-bug
git commit -am "fix: solve race condition"
```

```
git push rad HEAD:refs/patches
```

git提交的时候，会自动生成radicle patch对象并广播？怎么实现的？我在安装的时候，radicle改了git配置吗？

#### 其他节点更新

```
rad sync //同步seed的数据
```

查看patch

```
rad patch list
```

更新patch到本地，会在本地更新一个branch

```
rad patch checkout e5f0a5a
```

更新master

```
git merge patch/e5f0a5a
```

推送

```
git push rad master
```

### 节点和仓库的关系

一个仓库是对应多个节点（用户的），master分支应该是Delegate来维护

<img src="C:\Users\ASUS\AppData\Roaming\Typora\typora-user-images\image-20251011173327160.png" alt="image-20251011173327160" style="zoom:67%;" />

## 十、本地存储

目前本地存储方案待定，Client和Relay

信息存SQLITE

Vercel 了解下

答：

Vercel 更多的是托管页面，如果是后台，优先考虑docker部署

## 十一、开发计划

## 总结：实现建议阶段划分

| 模块/阶段              | 模块名称           | 功能              | 内容                      |
| ---------------------- | ------------------ | ----------------- | ------------------------- |
| 身份认证+网络传输      | Identity+Transport | Identity + QUIC   | 建立节点通信、公钥验证    |
| 节点管理+仓库管理+存储 | Storage            | Node+Repo+Storage | 建立并保存 Node/Repo 信息 |
| 消息广播+仓库路由      | Gossip             | Gossip + Routing  | 广播消息，建立路由表      |
| git兼容                | Git                | Git Sync          | 支持git fetch操作         |
| 开源协作               | Collaboration      | Issue+Patch       | 支持 Issue/Patch 协作     |
| 部署                   | Docker             | Docker            | 支持docker部署            |

看一下RustVault简易版能不能用->libvault

# 🦀 Rust 模块结构总览

```
p2p-git/
│
├── main.rs
│
├── identity/              # 身份与密钥层
│
├── transport/             # 网络传输层 (QUIC)
│
├── storage/               # 本地持久化 (通用) SQLite
│
├── gossip/                # Gossip 消息传播层
│
├── node/                  # 节点服务
│
├── git/                   # Git 对象传输层
│
├── activity/              # 协作层 (Issue / Patch )
│
├── cli/                   # 应用层命令行
│
└── utils/                 # 工具模块
 
```

