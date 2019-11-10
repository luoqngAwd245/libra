// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    chained_bft::{
        common::Payload,
        consensus_types::{block::Block, quorum_cert::QuorumCert},
        consensusdb::ConsensusDB,
        liveness::pacemaker_timeout_manager::HighestTimeoutCertificates,
        safety::safety_rules::ConsensusState,
    },
    consensus_provider::create_storage_read_client,
};
use config::config::NodeConfig;
use crypto::HashValue;
use failure::Result;
use logger::prelude::*;
use rmp_serde::{from_slice, to_vec_named};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// Persistent storage for liveness data
/// 活动数据的持久存储
pub trait PersistentLivenessStorage: Send + Sync {
    /// Persist the highest timeout certificate for improved liveness - proof for other replicas
    /// to jump to this round
    /// 坚持最高超时证书以提高活力 - 其他复制品的证据可以跳转到这一轮

    fn save_highest_timeout_cert(
        &self,
        highest_timeout_certs: HighestTimeoutCertificates,
    ) -> Result<()>;
}

/// Persistent storage is essential for maintaining safety when a node crashes.  Specifically,
/// upon a restart, a correct node will not equivocate.  Even if all nodes crash, safety is
/// guaranteed.  This trait also also supports liveness aspects (i.e. highest timeout certificate)
/// and supports clean up (i.e. tree pruning).
/// Blocks persisted are proposed but not yet committed.  The committed state is persisted
/// via StateComputer.
/// 持久存储对于节点崩溃时保持安全至关重要。 具体而言，在重新启动时，正确的节点将不会模糊。
/// 即使所有节点都崩溃，也能保证安全。 该特征还支持活跃方面（即最高超时证书）并支持清理（即树修剪）。
///
/// 持久化的块被提议但尚未提交。 已提交的状态通过StateComputer持久化。
pub trait PersistentStorage<T>: PersistentLivenessStorage + Send + Sync {
    /// Get an Arc to an instance of PersistentLivenessStorage
    /// (workaround for trait downcasting
    /// 获取PersistentLivenessStorage实例的Arc
    ///（特征向下转型的解决方法
    fn persistent_liveness_storage(&self) -> Box<dyn PersistentLivenessStorage>;

    /// Persist the blocks and quorum certs into storage atomically.
    /// 以原子方式将块和仲裁证书保存到存储中。
    fn save_tree(&self, blocks: Vec<Block<T>>, quorum_certs: Vec<QuorumCert>) -> Result<()>;

    /// Delete the corresponding blocks and quorum certs atomically.
    /// 原子地删除相应的块和仲裁证书。
    fn prune_tree(&self, block_ids: Vec<HashValue>) -> Result<()>;

    /// Persist the consensus state.
    /// 坚持共识状态。
    fn save_consensus_state(&self, state: ConsensusState) -> Result<()>;

    /// When the node restart, construct the instance and returned the data read from db.
    /// This could guarantee we only read once during start, and we would panic if the
    /// read fails.
    /// It makes sense to be synchronous since we can't do anything else until this finishes.
    /// 当节点重新启动时，构造实例并返回从db读取的数据。 这可以保证我们在启动时只读取一次，
    /// 如果读取失败，我们会感到恐慌。
    /// 同步是有意义的，因为在完成之前我们不能做任何其他事情。
    fn start(config: &NodeConfig) -> (Arc<Self>, RecoveryData<T>)
    where
        Self: Sized;
}

/// The recovery data constructed from raw consensusdb data, it'll find the root value and
/// blocks that need cleanup or return error if the input data is inconsistent.
/// 从原始的consensusdb数据构造的恢复数据，如果输入数据不一致，它将找到需要清理或返回错误的根值和块。
///
#[derive(Debug)]
pub struct RecoveryData<T> {
    // Safety data
    // 安全数据
    state: ConsensusState,
    root: (Block<T>, QuorumCert, QuorumCert),
    // 1. the blocks guarantee the topological ordering - parent <- child.
    // 2. all blocks are children of the root.
    // 1.块保证拓扑排序 - 父< - 子。
    // 2.所有块都是根的子节点。
    blocks: Vec<Block<T>>,
    quorum_certs: Vec<QuorumCert>,
    blocks_to_prune: Option<Vec<HashValue>>,

    // Liveness data
    // 活动数据
    highest_timeout_certificates: HighestTimeoutCertificates,

    // whether root is consistent with StateComputer, if not we need to do the state sync before
    // starting
    // root是否与StateComputer一致，如果不是，我们需要在启动之前进行状态同步
    need_sync: bool,
}

impl<T: Payload> RecoveryData<T> {
    pub fn new(
        state: ConsensusState,
        mut blocks: Vec<Block<T>>,
        mut quorum_certs: Vec<QuorumCert>,
        root_from_storage: HashValue,
        highest_timeout_certificates: HighestTimeoutCertificates,
    ) -> Result<Self> {
        let root =
            Self::find_root(&mut blocks, &mut quorum_certs, root_from_storage).map_err(|e| {
                format_err!(
                    "Blocks in db: {}\nQuorum Certs in db: {}, error: {}",
                    blocks
                        .iter()
                        .map(|b| format!("\n\t{}", b))
                        .collect::<Vec<String>>()
                        .concat(),
                    quorum_certs
                        .iter()
                        .map(|qc| format!("\n\t{}", qc))
                        .collect::<Vec<String>>()
                        .concat(),
                    e,
                )
            })?;
        let blocks_to_prune = Some(Self::find_blocks_to_prune(
            root.0.id(),
            &mut blocks,
            &mut quorum_certs,
        ));
        // if the root is different than the LI(S).block, we need to sync before start
        // 如果根与LI（S）.block不同，我们需要在开始之前同步
        let need_sync = root_from_storage != root.0.id();
        Ok(RecoveryData {
            state,
            root,
            blocks,
            quorum_certs,
            blocks_to_prune,
            highest_timeout_certificates,
            need_sync,
        })
    }

    pub fn state(&self) -> ConsensusState {
        self.state.clone()
    }

    pub fn take(
        self,
    ) -> (
        (Block<T>, QuorumCert, QuorumCert),
        Vec<Block<T>>,
        Vec<QuorumCert>,
    ) {
        (self.root, self.blocks, self.quorum_certs)
    }

    pub fn take_blocks_to_prune(&mut self) -> Vec<HashValue> {
        self.blocks_to_prune
            .take()
            .expect("blocks_to_prune already taken")
    }

    pub fn highest_timeout_certificates(&self) -> &HighestTimeoutCertificates {
        &self.highest_timeout_certificates
    }

    pub fn root_ledger_info(&self) -> QuorumCert {
        self.root.2.clone()
    }

    pub fn need_sync(&self) -> bool {
        self.need_sync
    }

    /// Finds the root (last committed block) and returns the root block, the QC to the root block
    /// and the ledger info for the root block, return an error if it can not be found.
    ///
    /// LI(S) is the highest known ledger info determined by storage.
    /// LI(C) is determined by ConsensusDB: it's the highest block id that is certified as committed
    /// by one of the QC's ledger infos.
    ///
    /// We guarantee a few invariants:
    /// 1. LI(C) must exist in blocks
    /// 2. LI(S).block.round <= LI(C).block.round
    ///
    /// We use the following condition to decide the root:
    /// 1. LI(S) exist && LI(S) is ancestor of LI(C) according to blocks, root = LI(S)
    /// 2. else root = LI(C)
    ///
    /// In a typical case, the QC certifying a commit of a block is persisted to ConsensusDB before
    /// this block is committed to the storage. Hence, ConsensusDB contains the
    /// block corresponding to LI(S) id, which is going to become the root.
    /// An additional complication is added in this code in order to tolerate a potential failure
    /// during state synchronization. In this case LI(S) might not be found in the blocks of
    /// ConsensusDB: we're going to start with LI(C) and invoke state synchronizer in order to
    /// resume the synchronization.
    ///
    /// 查找根（最后提交的块）并将根块，QC返回到根块以及根块的分类帐信息，如果找不到则返回错误。
    ///
    /// LI（S）是由存储确定的最高已知分类帐信息。
    /// LI（C）由ConsensusDB确定：它是由QC的分类账信息之一认证为最高的块ID。
    ///
    /// 我们保证一些不变量：
    /// 1. LI（C）必须以块的形式存在
    /// 2. LI（S）.block.round <= LI（C）.block.round
    ///
    /// 我们使用以下条件来确定根：
    /// 1. LI（S）存在&& LI（S）是LI（C）的祖先，根据块，root = LI（S）
    /// 2. else root = LI（C）
    ///
    /// 在典型情况下，在将此块提交到存储之前，证明块提交的QC将持久保存到ConsensusDB。 因此，
    /// ConsensusDB包含对应于LI（S）id的块，它将成为根。
    /// 此代码中添加了一个额外的复杂功能，以便在状态同步期间容忍潜在的故障。
    /// 在这种情况下，可能在ConsensusDB的块中找不到LI（S）：我们将从LI（C）开始并调用状态同步器以恢复同步。
    fn find_root(
        blocks: &mut Vec<Block<T>>,
        quorum_certs: &mut Vec<QuorumCert>,
        root_from_storage: HashValue,
    ) -> Result<(Block<T>, QuorumCert, QuorumCert)> {
        // sort by round to guarantee the topological order of parent <- child
        blocks.sort_by_key(Block::round);
        let root_from_consensus = {
            let id_to_round: HashMap<_, _> = blocks
                .iter()
                .map(|block| (block.id(), block.round()))
                .collect();
            let mut round_and_id = None;
            for qc in quorum_certs.iter() {
                if let Some(committed_block_id) = qc.committed_block_id() {
                    if let Some(round) = id_to_round.get(&committed_block_id) {
                        match round_and_id {
                            Some((r, _)) if r > round => (),
                            _ => round_and_id = Some((round, committed_block_id)),
                        }
                    }
                }
            }
            match round_and_id {
                Some((_, id)) => id,
                None => return Err(format_err!("No LI found in quorum certs.")),
            }
        };
        let root_id = {
            let mut tree = HashSet::new();
            tree.insert(root_from_storage);
            blocks.iter().for_each(|block| {
                if tree.contains(&block.parent_id()) {
                    tree.insert(block.id());
                }
            });
            if !tree.contains(&root_from_consensus) {
                root_from_consensus
            } else {
                root_from_storage
            }
        };

        let root_idx = blocks
            .iter()
            .position(|block| block.id() == root_id)
            .ok_or_else(|| format_err!("unable to find root: {}", root_id))?;
        let root_block = blocks.remove(root_idx);
        let root_quorum_cert = quorum_certs
            .iter()
            .find(|qc| qc.certified_block_id() == root_block.id())
            .ok_or_else(|| format_err!("No QC found for root: {}", root_id))?
            .clone();
        let root_ledger_info = quorum_certs
            .iter()
            .find(|qc| qc.committed_block_id() == Some(root_block.id()))
            .ok_or_else(|| format_err!("No LI found for root: {}", root_id))?
            .clone();
        Ok((root_block, root_quorum_cert, root_ledger_info))
    }

    fn find_blocks_to_prune(
        root_id: HashValue,
        blocks: &mut Vec<Block<T>>,
        quorum_certs: &mut Vec<QuorumCert>,
    ) -> Vec<HashValue> {
        // prune all the blocks that don't have root as ancestor
        // 修剪所有没有根作为祖先的块
        let mut tree = HashSet::new();
        let mut to_remove = vec![];
        tree.insert(root_id);
        // assume blocks are sorted by round already
        // 假设块已经按轮排序
        blocks.retain(|block| {
            if tree.contains(&block.parent_id()) {
                tree.insert(block.id());
                true
            } else {
                to_remove.push(block.id());
                false
            }
        });
        quorum_certs.retain(|qc| tree.contains(&qc.certified_block_id()));
        to_remove
    }
}

/// The proxy we use to persist data in libra db storage service via grpc.
/// 我们用于通过grpc在libra db存储服务中保存数据的代理。
pub struct StorageWriteProxy {
    db: Arc<ConsensusDB>,
}

impl StorageWriteProxy {
    pub fn new(db: Arc<ConsensusDB>) -> Self {
        StorageWriteProxy { db }
    }
}

impl PersistentLivenessStorage for StorageWriteProxy {
    fn save_highest_timeout_cert(
        &self,
        highest_timeout_certs: HighestTimeoutCertificates,
    ) -> Result<()> {
        self.db
            .save_highest_timeout_certificates(to_vec_named(&highest_timeout_certs)?)
    }
}

impl<T: Payload> PersistentStorage<T> for StorageWriteProxy {
    fn persistent_liveness_storage(&self) -> Box<dyn PersistentLivenessStorage> {
        Box::new(StorageWriteProxy::new(Arc::clone(&self.db)))
    }

    fn save_tree(&self, blocks: Vec<Block<T>>, quorum_certs: Vec<QuorumCert>) -> Result<()> {
        self.db
            .save_blocks_and_quorum_certificates(blocks, quorum_certs)
    }

    fn prune_tree(&self, block_ids: Vec<HashValue>) -> Result<()> {
        if !block_ids.is_empty() {
            // quorum certs that certified the block_ids will get removed
            self.db
                .delete_blocks_and_quorum_certificates::<T>(block_ids)?;
        }
        Ok(())
    }

    fn save_consensus_state(&self, state: ConsensusState) -> Result<()> {
        self.db.save_state(to_vec_named(&state)?)
    }

    fn start(config: &NodeConfig) -> (Arc<Self>, RecoveryData<T>) {
        info!("Start consensus recovery.");
        let read_client = create_storage_read_client(config);
        let db = Arc::new(ConsensusDB::new(config.storage.dir.clone()));
        let proxy = Arc::new(Self::new(Arc::clone(&db)));
        let initial_data = db.get_data().expect("unable to recover consensus data");
        let consensus_state = initial_data.0.map_or_else(ConsensusState::default, |s| {
            from_slice(&s[..]).expect("unable to deserialize consensus state")
        });
        debug!("Recovered consensus state: {}", consensus_state);
        let highest_timeout_certificates = initial_data
            .1
            .map_or_else(HighestTimeoutCertificates::default, |s| {
                from_slice(&s[..]).expect("unable to deserialize highest timeout certificates")
            });
        let mut blocks = initial_data.2;
        let mut quorum_certs: Vec<_> = initial_data.3;
        // bootstrap the empty store with genesis block and qc.
        // 用创世块和qc引导空存储。
        if blocks.is_empty() && quorum_certs.is_empty() {
            blocks.push(Block::make_genesis_block());
            quorum_certs.push(QuorumCert::certificate_for_genesis());
            proxy
                .save_tree(vec![blocks[0].clone()], vec![quorum_certs[0].clone()])
                .expect("unable to bootstrap the storage with genesis block");
        }
        let blocks_repr: Vec<String> = blocks.iter().map(|b| format!("\n\t{}", b)).collect();
        debug!(
            "The following blocks were restored from ConsensusDB : {}",
            blocks_repr.concat()
        );
        let qc_repr: Vec<String> = quorum_certs
            .iter()
            .map(|qc| format!("\n\t{}", qc))
            .collect();
        debug!(
            "The following blocks were restored from ConsensusDB: {}",
            qc_repr.concat()
        );

        // find the block corresponding to storage latest ledger info
        // 查找与存储最新分类帐信息相对应的块
        let (_, ledger_info, _) = read_client
            .update_to_latest_ledger(0, vec![])
            .expect("unable to read ledger info from storage");
        let root_from_storage = ledger_info.ledger_info().consensus_block_id();
        debug!(
            "The last committed block id as recorded in storage: {}",
            root_from_storage
        );

        let mut initial_data = RecoveryData::new(
            consensus_state,
            blocks,
            quorum_certs,
            root_from_storage,
            highest_timeout_certificates,
        )
        .unwrap_or_else(|e| panic!("Can not construct recovery data due to {}", e));

        <dyn PersistentStorage<T>>::prune_tree(proxy.as_ref(), initial_data.take_blocks_to_prune())
            .expect("unable to prune dangling blocks during restart");

        debug!("Consensus root to start with: {}", initial_data.root.0);

        if initial_data.need_sync {
            info!("Consensus recovery done but additional state synchronization is required.");
        } else {
            info!("Consensus recovery completed.")
        }
        (proxy, initial_data)
    }
}
