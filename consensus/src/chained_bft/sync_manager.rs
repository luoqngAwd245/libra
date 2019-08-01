// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    chained_bft::{
        block_storage::{BlockReader, BlockStore, InsertError},
        common::{Author, Payload},
        consensus_types::{block::Block, quorum_cert::QuorumCert, sync_info::SyncInfo},
        network::ConsensusNetworkImpl,
        persistent_storage::PersistentStorage,
    },
    counters,
    state_replication::StateComputer,
    state_synchronizer::SyncStatus,
    util::mutex_map::MutexMap,
};
use crypto::HashValue;
use failure::{self, Fail};
use logger::prelude::*;
use network::proto::BlockRetrievalStatus;
use rand::{prelude::*, Rng};
use std::{
    clone::Clone,
    result::Result,
    sync::Arc,
    time::{Duration, Instant},
};
use termion::color::*;
use types::{account_address::AccountAddress, transaction::TransactionListWithProof};

/// SyncManager is responsible for fetching dependencies and 'catching up' for given qc/ledger info
/// SyncManager负责获取依赖关系并为给定的qc /分类帐信息“追赶”
pub struct SyncManager<T> {
    block_store: Arc<BlockStore<T>>,
    storage: Arc<dyn PersistentStorage<T>>,
    network: ConsensusNetworkImpl,
    state_computer: Arc<dyn StateComputer<Payload = T>>,
    block_mutex_map: MutexMap<HashValue>,
}

/// Keeps the necessary context for `SyncMgr` to bring the missing information.
/// 此结构描述了我们要同步到的位置
pub struct SyncMgrContext {
    /// 用于调用状态同步的最高分类帐信息现在这是可选的，因为投票没有它
    pub highest_ledger_info: QuorumCert,
     /// 要插入块树的仲裁证书
    pub highest_quorum_cert: QuorumCert,
    /// Preferred peer: this is typically the peer that delivered the original QC and
    /// thus has higher chances to be able to return the information than the other
    /// peers that signed the QC.
    /// 触发此同步的消息的作者。
    /// 现在我们从这个同伴同步。 将来我们将使用quorum证书中的同行，而这个领域将主要是信息性的
    pub preferred_peer: Author,
}

impl SyncMgrContext {
    pub fn new(sync_info: &SyncInfo, preferred_peer: Author) -> Self {
        Self {
            highest_ledger_info: sync_info.highest_ledger_info().clone(),
            highest_quorum_cert: sync_info.highest_quorum_cert().clone(),
            preferred_peer,
        }
    }
}

impl<T> SyncManager<T>
where
    T: Payload,
{
    pub fn new(
        block_store: Arc<BlockStore<T>>,
        storage: Arc<dyn PersistentStorage<T>>,
        network: ConsensusNetworkImpl,
        state_computer: Arc<dyn StateComputer<Payload = T>>,
    ) -> SyncManager<T> {
        // Our counters are initialized via lazy_static, so they're not going to appear in
        // Prometheus if some conditions never happen.  Invoking get() function enforces creation.
        // 我们的计数器是通过lazy_static初始化的，所以如果某些条件永远不会发生，它们就不会出现在Prometheus中。
        // 调用get（）函数可以强制创建。
        counters::BLOCK_RETRIEVAL_COUNT.get();
        counters::STATE_SYNC_COUNT.get();
        counters::STATE_SYNC_TXN_REPLAYED.get();
        let block_mutex_map = MutexMap::new();
        SyncManager {
            block_store,
            storage,
            network,
            state_computer,
            block_mutex_map,
        }
    }

    /// Fetches dependencies for given sync_info.quorum_cert
    /// If gap is large, performs state sync using process_highest_ledger_info
    /// Inserts sync_info.quorum_cert into block store as the last step
    /// 获取给定sync_info.quorum_cert的依赖项
    /// 如果间隙很大，则使用process_highest_ledger_info执行状态同步
    /// 作为最后一步，将sync_info.quorum_cert插入到块存储中
    pub async fn sync_to(
        &mut self,
        deadline: Instant,
        sync_context: SyncMgrContext,
    ) -> failure::Result<()> {
        self.process_highest_ledger_info(
            sync_context.highest_ledger_info.clone(),
            sync_context.preferred_peer,
            deadline,
        )
        .await?;

        self.fetch_quorum_cert(
            sync_context.highest_quorum_cert.clone(),
            sync_context.preferred_peer,
            deadline,
        )
        .await?;
        Ok(())
    }

    /// Get a chunk of transactions as a batch
    /// 批量获取一大笔交易
    pub async fn get_chunk(
        &self,
        start_version: u64,
        target_version: u64,
        batch_size: u64,
    ) -> failure::Result<TransactionListWithProof> {
        self.state_computer
            .get_chunk(start_version, target_version, batch_size)
            .await
    }

    pub async fn execute_and_insert_block(
        &self,
        block: Block<T>,
    ) -> Result<Arc<Block<T>>, InsertError> {
        let _guard = self.block_mutex_map.lock(block.id());
        // execute_and_insert_block has shortcut to return block if it exists
        self.block_store.execute_and_insert_block(block).await
    }

    /// Insert the quorum certificate separately from the block, used to split the processing of
    /// updating the consensus state(with qc) and deciding whether to vote(with block)
    /// The missing ancestors are going to be retrieved from the given peer. If a given peer
    /// fails to provide the missing ancestors, the qc is not going to be added.
    /// 从块中单独插入仲裁证书，用于拆分更新共识状态（使用qc）和决定是否投票（使用块）的处理
    /// 将丢失的祖先将从给定的同伴中检索。 如果给定的对等方未能提供缺失的祖先，则不会添加qc。
    pub async fn fetch_quorum_cert(
        &self,
        qc: QuorumCert,
        preferred_peer: Author,
        deadline: Instant,
    ) -> Result<(), InsertError> {
        let mut lock_set = self.block_mutex_map.new_lock_set();
        let mut pending = vec![];
        let network = self.network.clone();
        let mut retriever = BlockRetriever {
            network,
            deadline,
            preferred_peer,
        };
        let mut retrieve_qc = qc.clone();
        loop {
            if lock_set
                .lock(retrieve_qc.certified_block_id())
                .await
                .is_err()
            {
                // This should not be possible because that would mean we have circular
                // dependency between signed blocks
                panic!(
                    "Can not re-acquire lock for block {} during fetch_quorum_cert",
                    retrieve_qc.certified_block_id()
                );
            }
            if self
                .block_store
                .block_exists(retrieve_qc.certified_block_id())
            {
                break;
            }
            let mut blocks = retriever.retrieve_block_for_qc(&retrieve_qc, 1).await?;
            // retrieve_block_for_qc guarantees that blocks has exactly 1 element
            let block = blocks.remove(0);
            retrieve_qc = block.quorum_cert().clone();
            pending.push(block);
        }
        // insert the qc <- block pair
        while let Some(block) = pending.pop() {
            let block_qc = block.quorum_cert().clone();
            self.block_store.insert_single_quorum_cert(block_qc).await?;
            self.block_store.execute_and_insert_block(block).await?;
        }
        self.block_store.insert_single_quorum_cert(qc).await
    }

    /// Check the highest ledger info sent by peer to see if we're behind and start a fast
    /// forward sync if the committed block doesn't exist in our tree.
    /// It works as follows:
    /// 1. request the committed 3-chain from the peer, if C2 is the highest_ledger_info
    /// we request for B0 <- C0 <- B1 <- C1 <- B2 (<- C2)
    /// 2. We persist the 3-chain to storage before start sync to ensure we could restart if we
    /// crash in the middle of the sync.
    /// 3. We prune the old tree and replace with a new tree built with the 3-chain.
    ///  检查对等方发送的最高分类帐信息，看看我们是否落后并且如果我们的树中不存在已提交的块，则启动快进同步。
    /// 它的工作原理如下：
    /// 1.请求来自对等方的已提交3链，如果C2是我们请求B0 < -  C0 < -  B1 < -  C1 < -  B2（< -  C2）的highest_ledger_info
    /// 2.我们在开始同步之前将3链保留到存储，以确保如果我们在同步过程中崩溃，我们可以重新启动。
    /// 我们修剪旧树，并用一个用3链建造的新树替换。
    async fn process_highest_ledger_info(
        &self,
        highest_ledger_info: QuorumCert,
        peer: Author,
        deadline: Instant,
    ) -> failure::Result<()> {
        let committed_block_id = highest_ledger_info
            .committed_block_id()
            .ok_or_else(|| format_err!("highest ledger info has no committed block"))?;
        if !self
            .block_store
            .need_sync_for_quorum_cert(committed_block_id, &highest_ledger_info)
        {
            return Ok(());
        }
        debug!(
            "Start state sync with peer: {}, to block: {}, round: {} from {}",
            peer.short_str(),
            committed_block_id,
            highest_ledger_info.certified_block_round() - 2,
            self.block_store.root()
        );
        let network = self.network.clone();
        let mut retriever = BlockRetriever {
            network,
            deadline,
            preferred_peer: peer,
        };
        let mut blocks = retriever
            .retrieve_block_for_qc(&highest_ledger_info, 3)
            .await?;
        assert_eq!(
            blocks.last().expect("should have 3-chain").id(),
            committed_block_id
        );
        let mut quorum_certs = vec![];
        quorum_certs.push(highest_ledger_info.clone());
        quorum_certs.push(blocks[0].quorum_cert().clone());
        quorum_certs.push(blocks[1].quorum_cert().clone());
        // If a node restarts in the middle of state synchronization, it is going to try to catch up
        // to the stored quorum certs as the new root.
         // 如果节点在状态同步过程中重新启动，它将尝试捕获存储的仲裁证书作为新根。
        self.storage
            .save_tree(blocks.clone(), quorum_certs.clone())?;
        let pre_sync_instance = Instant::now();
        match self
            .state_computer
            .sync_to(highest_ledger_info.clone())
            .await
        {
            Ok(SyncStatus::Finished) => (),
            Ok(e) => panic!(
                "state synchronizer failure: {:?}, this validator will be killed as it can not \
                 recover from this error.  After the validator is restarted, synchronization will \
                 be retried.",
                e
            ),
            Err(e) => panic!(
                "state synchronizer failure: {:?}, this validator will be killed as it can not \
                 recover from this error.  After the validator is restarted, synchronization will \
                 be retried.",
                e
            ),
        };
        counters::STATE_SYNC_DURATION_MS.observe(pre_sync_instance.elapsed().as_millis() as f64);
        let root = (
            blocks.pop().expect("should have 3-chain"),
            quorum_certs.last().expect("should have 3-chain").clone(),
            highest_ledger_info.clone(),
        );
        debug!("{}Sync to{} {}", Fg(Blue), Fg(Reset), root.0);
        // ensure it's [b1, b2]
        blocks.reverse();
        self.block_store.rebuild(root, blocks, quorum_certs).await;
        Ok(())
    }
}

/// BlockRetriever is used internally to retrieve blocks
/// BlockRetriever在内部用于检索块
struct BlockRetriever {
    network: ConsensusNetworkImpl,
    deadline: Instant,
    preferred_peer: Author,
}

#[derive(Debug, Fail)]
enum BlockRetrieverError {
    #[fail(display = "All peers failed")]
    AllPeersFailed,
    #[fail(display = "Round deadline reached")]
    RoundDeadlineReached,
}

impl From<BlockRetrieverError> for InsertError {
    fn from(_error: BlockRetrieverError) -> Self {
        InsertError::AncestorRetrievalError
    }
}

impl BlockRetriever {
    /// Retrieve chain of n blocks for given QC
    ///
    /// Returns Result with Vec that has a guaranteed size of num_blocks
    /// This guarantee is based on BlockRetrievalResponse::verify that ensures that number of
    /// blocks in response is equal to number of blocks requested.  This method will
    /// continue until either the round deadline is reached or the quorum certificate members all
    /// fail to return the missing chain.
    ///
    /// The first attempt of block retrieval will always be sent to preferred_peer to allow the
    /// leader to drive quorum certificate creation The other peers from the quorum certificate
    /// will be randomly tried next.  If all members of the quorum certificate are exhausted, an
    /// error is returned
    ///
    /// 检索给定QC的n个块的链
    ///
    /// 返回具有保证大小num_blocks的Vec的结果
    /// 此保证基于BlockRetrievalResponse :: verify，确保响应中的块数等于请求的块数。 此方法将继续，
    /// 直到达到轮次截止日期或仲裁证书成员都未能返回丢失的链。
    ///
    /// 块检索的第一次尝试将始终发送到preferred_peer以允许领导者驱动仲裁证书创建接下来将随机尝试
    /// 仲裁证书中的其他对等。 如果仲裁证书的所有成员都已用尽，则会返回错误
    pub async fn retrieve_block_for_qc<'a, T>(
        &'a mut self,
        qc: &'a QuorumCert,
        num_blocks: u64,
    ) -> Result<Vec<Block<T>>, BlockRetrieverError>
    where
        T: Payload,
    {
        let block_id = qc.certified_block_id();
        let mut peers: Vec<&AccountAddress> = qc.ledger_info().signatures().keys().collect();
        let mut attempt = 0_u32;
        loop {
            if peers.is_empty() {
                warn!(
                    "Failed to fetch block {} in {} attempts: no more peers available",
                    block_id, attempt
                );
                return Err(BlockRetrieverError::AllPeersFailed);
            }
            let peer = self.pick_peer(attempt, &mut peers);
            attempt += 1;

            let timeout = retrieval_timeout(&self.deadline, attempt);
            let timeout = if let Some(timeout) = timeout {
                timeout
            } else {
                warn!("Failed to fetch block {} from {}, attempt {}: round deadline was reached, won't make more attempts", block_id, peer, attempt);
                return Err(BlockRetrieverError::RoundDeadlineReached);
            };
            debug!(
                "Fetching {} from {}, attempt {}",
                block_id,
                peer.short_str(),
                attempt
            );
            let response = self
                .network
                .request_block(block_id, num_blocks, peer, timeout)
                .await;
            let response = match response {
                Err(e) => {
                    warn!(
                        "Failed to fetch block {} from {}: {:?}, trying another peer",
                        block_id,
                        peer.short_str(),
                        e
                    );
                    continue;
                }
                Ok(response) => response,
            };
            if response.status != BlockRetrievalStatus::SUCCEEDED {
                warn!(
                    "Failed to fetch block {} from {}: {:?}, trying another peer",
                    block_id,
                    peer.short_str(),
                    response.status
                );
                continue;
            }
            return Ok(response.blocks);
        }
    }

    fn pick_peer(&self, attempt: u32, peers: &mut Vec<&AccountAddress>) -> AccountAddress {
        assert!(!peers.is_empty(), "pick_peer on empty peer list");

        if attempt == 0 {
            // remove preferred_peer if its in list of peers
            // (strictly speaking it is not required to be there)
            for i in 0..peers.len() {
                if *peers[i] == self.preferred_peer {
                    peers.remove(i);
                    break;
                }
            }
            return self.preferred_peer;
        }
        let peer_idx = thread_rng().gen_range(0, peers.len());
        *peers.remove(peer_idx)
    }
}

// Max timeout is 16s=RETRIEVAL_INITIAL_TIMEOUT*(2^RETRIEVAL_MAX_EXP)
const RETRIEVAL_INITIAL_TIMEOUT: Duration = Duration::from_secs(1);
const RETRIEVAL_MAX_EXP: u32 = 4;

/// Returns exponentially increasing timeout with
/// limit of RETRIEVAL_INITIAL_TIMEOUT*(2^RETRIEVAL_MAX_EXP)
/// 返回指数增加的超时，限制为RETRIEVAL_INITIAL_TIMEOUT *（2 ^ RETRIEVAL_MAX_EXP）
fn retrieval_timeout(deadline: &Instant, attempt: u32) -> Option<Duration> {
    assert!(attempt > 0, "retrieval_timeout attempt can't be 0");
    let exp = RETRIEVAL_MAX_EXP.min(attempt - 1); // [0..RETRIEVAL_MAX_EXP]
    let request_timeout = RETRIEVAL_INITIAL_TIMEOUT * 2_u32.pow(exp);
    let now = Instant::now();
    let deadline_timeout = if *deadline >= now {
        Some(deadline.duration_since(now))
    } else {
        None
    };
    deadline_timeout.map(|delay| request_timeout.min(delay))
}
