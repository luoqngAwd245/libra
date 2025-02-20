// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use config::config::NodeConfig;
use failure::prelude::*;
use network::validator_network::{ConsensusNetworkEvents, ConsensusNetworkSender};

use crate::chained_bft::chained_bft_consensus_provider::ChainedBftProvider;
use execution_proto::proto::execution_grpc::ExecutionClient;
use grpcio::{ChannelBuilder, EnvBuilder};
use mempool::proto::mempool_grpc::MempoolClient;
use std::sync::Arc;
use storage_client::{StorageRead, StorageReadServiceClient};

/// Public interface to a consensus protocol.
/// 一致性协议公共接口
pub trait ConsensusProvider {
    /// Spawns new threads, starts the consensus operations (retrieve txns, consensus protocol,
    /// execute txns, commit txns, update txn status in the mempool, etc).
    /// The function returns after consensus has recovered its initial state,
    /// and has established the required connections (e.g., to mempool and
    /// executor).
    /// spawn 新的线程，启动一致性操作（取回交易，一致性协议， 提交交易，更新交易状态在内存池等等）
    /// 该函数在共识恢复其初始状态后返回，并已建立所需的连接（例如，到mempool和executor）。
    fn start(&mut self) -> Result<()>;

    /// Stop the consensus operations. The function returns after graceful shutdown.
    ///终止一致性操作。该函数在正常关闭后返回。
    fn stop(&mut self);
}

/// Helper function to create a ConsensusProvider based on configuration
/// Helper功能可根据配置创建ConsensusProvider
pub fn make_consensus_provider(
    node_config: &mut NodeConfig,
    network_sender: ConsensusNetworkSender,
    network_receiver: ConsensusNetworkEvents,
) -> Box<dyn ConsensusProvider> {
    Box::new(ChainedBftProvider::new(
        node_config,
        network_sender,
        network_receiver,
        create_mempool_client(node_config),
        create_execution_client(node_config),
    ))
}
/// Create a mempool client assuming the mempool is running on localhost
/// 假设mempool在localhost上运行，创建一个mempool客户端
fn create_mempool_client(config: &NodeConfig) -> Arc<MempoolClient> {
    let port = config.mempool.mempool_service_port;
    let connection_str = format!("localhost:{}", port);

    let env = Arc::new(EnvBuilder::new().name_prefix("grpc-con-mem-").build());
    Arc::new(MempoolClient::new(
        ChannelBuilder::new(env).connect(&connection_str),
    ))
}

/// Create an execution client assuming the mempool is running on localhost
/// 假设mempool在localhost上运行，创建一个执行客户端
fn create_execution_client(config: &NodeConfig) -> Arc<ExecutionClient> {
    let connection_str = format!("localhost:{}", config.execution.port);

    let env = Arc::new(EnvBuilder::new().name_prefix("grpc-con-exe-").build());
    Arc::new(ExecutionClient::new(
        ChannelBuilder::new(env).connect(&connection_str),
    ))
}

/// Create a storage read client based on the config
/// 根据配置创建存储读取客户端
pub fn create_storage_read_client(config: &NodeConfig) -> Arc<dyn StorageRead> {
    let env = Arc::new(EnvBuilder::new().name_prefix("grpc-con-sto-").build());
    Arc::new(StorageReadServiceClient::new(
        env,
        &config.storage.address,
        config.storage.port,
    ))
}
