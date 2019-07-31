---
id: consensus
title: Consensus
custom_edit_url: https://github.com/libra/libra/edit/master/consensus/README.md
---

# Consensus

The consensus component supports state machine replication using the LibraBFT consensus protocol.
共识模块使用 libraBFT共识协议支持状态机复制

## Overview

A consensus protocol allows a set of validators to create the logical appearance of a single database. The consensus protocol replicates submitted transactions among the validators, executes potential transactions against the current database, and then agrees on a binding commitment to the ordering of transactions and resulting execution. As a result, all validators can maintain an identical database for a given version number following the [state machine replication paradigm](https://dl.acm.org/citation.cfm?id=98167). The Libra protocol uses a variant of the [HotStuff consensus protocol](https://arxiv.org/pdf/1803.05069.pdf), a recent Byzantine fault-tolerant ([BFT](https://en.wikipedia.org/wiki/Byzantine_fault)) consensus protocol, called LibraBFT. It provides safety (all honest validators agree on commits and execution) and liveness (commits are continually produced) in the partial synchrony model defined in the paper "Consensus in the Presence of Partial Synchrony" by Dwork, Lynch, and Stockmeyer ([DLS](https://groups.csail.mit.edu/tds/papers/Lynch/jacm88.pdf)) and mentioned in the paper ["Practical Byzantine Fault Tolerance" (PBFT)](http://pmg.csail.mit.edu/papers/osdi99.pdf) by Castro and Liskov, as well as newer protocols such as [Tendermint](https://arxiv.org/abs/1807.04938). In this document, we present a high-level description of the LibraBFT protocol and discuss how the code is organized. Refer to the [Libra Blockchain Paper](https://developers.libra.org/docs/the-libra-blockchain-paper) to learn more about how LibraBFT fits into the Libra protocol. For details on the specifications and proofs of LibraBFT, read the full [technical report](https://developers.libra.org/docs/state-machine-replication-paper).
共识协议允许一组验证器来创建单数据库的逻辑模型。共识协议在验证器之间复制提交的交易，在当前的数据库基础上执行一些待加入的交易，以及对交易排序和
执行结果达成一致。因此，所有的验证器都可以在 状态机复制规范 之下对于给定的版本号维护相同的数据库。 Libra 区块链使用 HotStuff 共识协议 的变体, 
这是最新的一种拜占庭容错 (BFT) 共识协议, 称为LibraBFT。在Dwork，Lynch和Stockmeyer(DLS) 的论文（"Consensus in the Presence of Partial Synchrony"）
中论述了在部分同步模型中也提供安全性（所有诚实的验证者会确认提案和执行）和存活性（可持续产生提案），这个共识在新协议 PBFT （如 Tendermint ）
中也使用了。 在本文档中，我们从高层次描述 LibraBFT协议，并讨论代码的组成。 请参阅这篇论文 了解LibraBFT如何配合Libra区块链。 有关LibraBFT的规范
和证明的详细信息，可以阅读完整的技术论文.

Agreement on the database state must be reached between validators, even if
there are Byzantine faults. The Byzantine failures model allows some validators
to arbitrarily deviate from the protocol without constraint, with the exception
of being computationally bound (and thus not able to break cryptographic assumptions). Byzantine faults are worst-case errors where validators collude and behave maliciously to try to sabotage system behavior. A consensus protocol that tolerates Byzantine faults caused by malicious or hacked validators can also mitigate arbitrary hardware and software failures.
即使存在拜占庭式故障，也必须在验证者之间达成数据库状态的一致，拜占庭故障模型允许一些验证器在没有约束的情况下随意偏离协议，不过依然会有计算限制
（假设密码学是无法破解）。拜占庭故障最坏情况是其中验证者串通并且试图恶意破坏系统一致性。拜占庭容错的共识协议，则能容忍由恶意或被黑客控制的验证器
引起的问题，缓解相关的硬件和软件故障。
LibraBFT assumes that a set of 3f + 1 votes is distributed among a set of validators that may be honest or Byzantine. LibraBFT remains safe, preventing attacks such as double spends and forks when at most f votes are controlled by Byzantine validators &mdash; also implying that at least 2f+1 votes are honest.  LibraBFT remains live, committing transactions from clients, as long as there exists a global stabilization time (GST), after which all messages between honest validators are delivered to other honest validators within a maximal network delay $\Delta$ (this is the partial synchrony model introduced in [DLS](https://groups.csail.mit.edu/tds/papers/Lynch/jacm88.pdf)). In addition to traditional guarantees, LibraBFT maintains safety when validators crash and restart — even if all validators restart at the same time.
LibraBFT前提假设一组有 3f + 1 的投票分布在一组验证器中，这些验证器可能是诚实的，也可能是拜占庭式（恶意）的。 不超过 f 票是由拜占庭验证器控制的话（也就意味着至少有 2f+1 票是诚实的），这时候LibraBFT仍然是安全的，能够阻止如双花和分叉的攻击。只要存在全局稳定时间（GST），LibraBFT就会保持在线，客户端可提交交易，所有消息都会在最大网络延迟 
Δ\Delta
Δ 内在诚实的验证器之间得到传递，(在DLS 论文 中有介绍）。除了传统的保证之外，LibraBFT在验证器崩溃和重启时仍然保持安全 - 即使所有验证器同时重启

### LibraBFT Overview libraBFT 概要

In LibraBFT, validators receive transactions from clients and share them with each other through a shared mempool protocol. The LibraBFT protocol then proceeds in a sequence of rounds. In each round, a validator takes the role of leader and proposes a block of transactions to extend a certified sequence of blocks (see quorum certificates below) that contain the full previous transaction history. A validator receives the proposed block and checks their voting rules to determine if it should vote for certifying this block. These simple rules ensure the safety of LibraBFT — and their implementation can be cleanly separated and audited. If the validator intends to vote for this block, it executes the block’s transactions speculatively and without external effect. This results in the computation of an authenticator for the database that results from the execution of the block. The validator then sends a signed vote for the block and the database authenticator to the leader. The leader gathers these votes to form a quorum certificate that provides evidence of $\ge$ 2f + 1 votes for this block and broadcasts the quorum certificate to all validators.
在LibraBFT中，验证器接收来自客户机的交易，并通过共享的内存池协议彼此共享。然后LibraBFT协议按回合轮次进行。每一个回合，一个验证者会扮演领导者的角色，并提案一个交易区块，以扩展经过认证（请参考下面介绍的法定证明人数投票）的块序列，这个快序列里包含先前完整的交易历史。验证器接收提议的块并检查其投票规则，以确定是否应该投票认证该块。这些简单的规则确保了LibraBFT的安全性，并且它们的实现可以被清晰地分离和审计。如果验证器打算为该块投票，它会以推测方式（speculatively）执行块的交易，而不会产生外部影响。数据库的验证器（authenticator）的计算结果和区块的执行结果一致的话，然后验证器把对块的签名投票和数据库验证者（authenticator）发送给领导者。领导者收集这些投票来生成一个超过法定
≥\ge
≥ 2f + 1 证明人数的投票证明，并将法定人数证明广播给所有验证人。
A block is committed when a contiguous 3-chain commit rule is met. A block at round k is committed if it has a quorum certificate and is confirmed by two more blocks and quorum certificates at rounds k + 1 and k + 2. The commit rule eventually allows honest validators to commit a block. LibraBFT guarantees that all honest validators will eventually commit the block (and proceeding sequence of blocks linked from it). Once a sequence of blocks has committed, the state resulting from executing their transactions can be persisted and forms a replicated database.
当连续三次在链上提交提议满足规则，将区块会的到确认，即如果这个块（假设为 k 回合的块）具有法定人数证明并且在其后2个回合 k+1 和 k+2 也具有法定人数证明，则第k轮的块得到确认。提交规则最终允许诚实的验证器提交块。 LibraBFT保证所有诚实的验证器最终都会提交块（并且延长之前链接的块序列）。 一旦块被提交确认，执行交易后的结果状态就会永久存储，并形成一个复制数据库。


### Advantages of the HotStuff Paradigm HotStuff的优势

We evaluated several BFT-based protocols against the dimensions of performance, reliability, security, ease of robust implementation, and operational overhead for validators. Our goal was to choose a protocol that would initially support at least 100 validators and would be able to evolve over time to support 500–1,000 validators. We had three reasons for selecting the HotStuff protocol as the basis for LibraBFT: (i) simplicity and modularity; (ii) ability to easily integrate consensus with execution; and (iii) promising performance in early experiments.
我们从性能、可靠性、安全性、健壮实现的简易性以及验证器操作开销几个维度评估了几种基于BFT的协议。我们的目标是选择初期支持至少100个验证器的协议，并且它能够随着时间的推移演进到可支持500-1000个验证器。 选择HotStuff协议作为LibraBFT的基础有三个原因： (i) 简单和模块化; (ii) 方便将共识与执行集成的能力; (iii) 在早期实验中表现良好。


The HotStuff protocol decomposes into modules for safety (voting and commit rules) and liveness (pacemaker). This decoupling provides the ability to develop and experiment independently and on different modules in parallel. Due to the simple voting and commit rules, protocol safety is easy to implement and verify. It is straightforward to integrate execution as a part of consensus to avoid forking issues that arise from non-deterministic execution in a leader-based protocol. Finally, our early prototypes confirmed high throughput and low transaction latency as independently measured in [HotStuff]((https://arxiv.org/pdf/1803.05069.pdf)). We did not consider proof-of-work based protocols, such as [Bitcoin](https://bitcoin.org/bitcoin.pdf), due to their poor performance
and high energy (and environmental) costs.
HotStuff协议分解为安全模块（投票和提交规则）和存活模块（“复活起搏器”）。 这种解耦提供了开发和实验两套可独立并行运行环境的能力。 由于简单的投票和提交规则，协议安全性易于实现和验证。 将执行作为共识的一部分进行集成也是很自然的，这可以避免基于领导的协议的非确定性执行而产生分叉的问题。 最后，我们的早期原型也确认了HotStuff 满足高吞吐量和低交易延迟（独立的检测），我们没有考虑基于工作量证明的协议，例如 Bitcoin, 因为它们低性能和高能耗（以及环境）成本。


### HotStuff Extensions and Modifications HotStuff 扩展和修改

In LibraBFT, to better support the goals of the Libra ecosystem, we extend and adapt the core HotStuff protocol and implementation in several ways. Importantly, we reformulate the safety conditions and provide extended proofs of safety, liveness, and optimistic responsiveness. We also implement a number of additional features. First, we make the protocol more resistant to non-determinism bugs, by having validators collectively sign the resulting state of a block rather than just the sequence of transactions. This also allows clients to use quorum certificates to authenticate reads from the database. Second, we design a pacemaker that emits explicit timeouts, and validators rely on a quorum of those to move to the next round — without requiring synchronized clocks. Third, we intend to design an unpredictable leader election mechanism in which the leader of a round is determined by the proposer of the latest committed block using a verifiable random function [VRF](https://people.csail.mit.edu/silvio/Selected%20Scientific%20Papers/Pseudo%20Randomness/Verifiable_Random_Functions.pdf). This mechanism limits the window of time in which an adversary can launch an effective denial-of-service attack against a leader. Fourth, we use aggregate signatures that preserve the identity of validators who sign quorum certificates. This allows us to provide incentives to validators that contribute to quorum certificates. Aggregate signatures also do not require a complex [threshold key setup](https://www.cypherpunks.ca/~iang/pubs/DKG.pdf).
在LibraBFT中，为了更好地支持Libra生态系统的目标，我们以多种方式扩展和调整了核心HotStuff协议和实现。重要的是，我们重新定义了安全条件，并提供了安全、存活度和更高响应度的扩展证明。我们还实现了一些附加功能。首先，通过让验证器对块的结果状态(而不仅仅是交易序列)进行集体签名，我们使协议更能抵抗非确定性错误。 还允许客户端使用法定人数证书来验证读取的数据库。 其次，我们设计了一个发出明确超时的起搏器，验证器依靠法定人数来进入下一轮 - 不需要同步时钟。 第三，我们打算设计一个不可预测的领导者选举机制，其中一轮的领导者由最新提交的块的提议者使用可验证的随机函数VRF 确定。 这种机制限制了攻击者可以针对领导者发起有效拒绝服务攻击的时间窗口。 第四，我们使用聚合签名来保留签署仲裁证书的验证者的身份。 这使我们能够为有助于仲裁证书的验证人提供激励。 聚合签名也不需要复杂的 密钥阈值设置.


## Implementation Details 实现细节

共识组件主要在 Actor 程序模块中实现 — 即，它使用消息传递在不同的子组件之间进行通信，其中 tokio 框架用作任务运行时。actor模型的主要例外是(因为它是由几个子组件并行访问的)是共识数据结构 BlockStore ，它管理块、执行、仲裁证书和其他共享数据结构。共识组件中的主要子组件是：
* **TxnManager** is the interface to the mempool component and supports the pulling of transactions as well as removing committed transactions. A proposer uses on-demand pull transactions from mempool to form a proposal block.
* TxnManager 是内存池组件的接口，支持拉取交易以及删除已提交的交易。 提议者使用来自内存池中的按需拉取交易来形成提议块。
* **StateComputer** is the interface for accessing the execution component. It can execute blocks, commit blocks, and can synchronize state.
* StateComputer 是访问执行组件的接口。 它可以执行块，提交块，并可以同步状态。
* **BlockStore** maintains the tree of proposal blocks, block execution, votes, quorum certificates, and persistent 
 storage. It is responsible for maintaining the consistency of the combination of these data structures and can be concurrently accessed by other subcomponents. 
* BlockStore 维护提议块树，块执行，投票，仲裁证书和持久存储。 它负责维护这些数据结构组合的一致性，并且可以由其他子组件同时访问。
* **EventProcessor** is responsible for processing the individual events (e.g., process_new_round, process_proposal, 
process_vote). It exposes the async processing functions for each event type and drives the protocol.
* EventProcessor 负责处理各个事件 (例如, process_new_round, process_proposal, process_vote). 它公开每个事件类型的异步处理函数和驱动协议。
* **Pacemaker** is responsible for the liveness of the consensus protocol. It changes rounds due to timeout certificates or quorum certificates and proposes blocks when it is the proposer for the current round.
* Pacemaker 负责共识协议的活跃性。 它由于超时证书或仲裁证书而改变轮次，并在它是当前轮次的提议者时提出阻止。
* **SafetyRules** is responsible for the safety of the consensus protocol. It processes quorum certificates and LedgerInfo to learn about new commits and guarantees that the two voting rules are followed &mdash; even in the case of restart (since all safety data is persisted to local storage). 
* SafetyRules 负责共识协议的安全性。 它处理仲裁证书和分类信息以了解新的提交，并保证遵循两个投票规则 — 即使在重启的情况下（因为所有安全数据都持久保存到本地存储）。


All consensus messages are signed by their creators and verified by their receivers. Message verification occurs closest to the network layer to avoid invalid or unnecessary data from entering the consensus protocol.

## How is this module organized?

    consensus
    ├── src
    │   └── chained_bft                # Implementation of the LibraBFT protocol
    │       ├── block_storage          # In-memory storage of blocks and related data structures
    │       ├── consensus_types        # Consensus data types (i.e. quorum certificates)
    │       ├── consensusdb            # Database interaction to persist consensus data for safety and liveness
    │       ├── liveness               # Pacemaker, proposer, and other liveness related code
    │       ├── safety                 # Safety (voting) rules
    │       └── test_utils             # Mock implementations that are used for testing only
    └── state_synchronizer             # Synchronization between validators to catch up on committed state
