// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::admission_control_service::AdmissionControlService;
use admission_control_proto::proto::admission_control_grpc;
use config::config::NodeConfig;
use debug_interface::{node_debug_service::NodeDebugService, proto::node_debug_interface_grpc};
use failure::prelude::*;
use grpc_helpers::spawn_service_thread;
use grpcio::{ChannelBuilder, EnvBuilder, Environment};
use logger::prelude::*;
use mempool::proto::{mempool_client::MempoolClientTrait, mempool_grpc::MempoolClient};
use std::{sync::Arc, thread};
use storage_client::{StorageRead, StorageReadServiceClient};
use vm_validator::vm_validator::VMValidator;

/// Struct to run Admission Control service in a dedicated process. It will be used to spin up
/// extra AC instances to talk to the same validator.
/// 结构体跑AC 服务在一个专用进程。他被用来旋转一个额外的AC实例用来和相同的验证者通信
pub struct AdmissionControlNode {
    /// Config used to setup environment for this Admission Control service instance.
    /// 用来为AC服务实例建立环境的配置
    node_config: NodeConfig,
}

impl Drop for AdmissionControlNode {
    fn drop(&mut self) {
        info!("Drop AdmissionControl node");
    }
}

impl AdmissionControlNode {
    /// Construct a new AdmissionControlNode instance using NodeConfig.
    /// 用节点配置构建一个AC节点的实例
    pub fn new(node_config: NodeConfig) -> Self {
        AdmissionControlNode { node_config }
    }

    /// Setup environment and start a new Admission Control service.
    /// 建立环境变量 并启动一个新的AC服务
    pub fn run(&self) -> Result<()> {
        logger::set_global_log_collector(
            self.node_config
                .log_collector
                .get_log_collector_type()
                .unwrap(),
            self.node_config.log_collector.is_async,
            self.node_config.log_collector.chan_size,
        );
        info!("Starting AdmissionControl node",);
        // Start receiving requests
        // 启动接收请求
        let client_env = Arc::new(EnvBuilder::new().name_prefix("grpc-ac-mem-").build());
        let mempool_connection_str = format!(
            "{}:{}",
            self.node_config.mempool.address, self.node_config.mempool.mempool_service_port
        );
        let mempool_channel =
            ChannelBuilder::new(Arc::clone(&client_env)).connect(&mempool_connection_str);

        self.run_with_clients(
            Arc::clone(&client_env),
            Arc::new(MempoolClient::new(mempool_channel)),
            Some(Arc::new(StorageReadServiceClient::new(
                Arc::clone(&client_env),
                &self.node_config.storage.address,
                self.node_config.storage.port,
            ))),
        )
    }

    /// This method will start a node using the provided clients to external services.
    /// For now, mempool is a mandatory argument, and storage is Option. If it doesn't exist,
    /// it'll be generated before starting the node.
    /// 此方法将使用提供的客户端启动节点到外部服务。目前，mempool是必需参数，存储是可選的。
    /// 如果不存在，在节点启动之前生成。
    pub fn run_with_clients<M: MempoolClientTrait + 'static>(
        &self,
        env: Arc<Environment>,
        mp_client: Arc<M>,
        storage_client: Option<Arc<StorageReadServiceClient>>,
    ) -> Result<()> {
        // create storage client if doesn't exist 创建存储客户端如果不存在
        let storage_client: Arc<dyn StorageRead> = match storage_client {
            Some(c) => c,
            None => Arc::new(StorageReadServiceClient::new(
                env,
                &self.node_config.storage.address,
                self.node_config.storage.port,
            )),
        };

        let vm_validator = Arc::new(VMValidator::new(
            &self.node_config,
            Arc::clone(&storage_client),
        ));

        let handle = AdmissionControlService::new(
            mp_client,
            storage_client,
            vm_validator,
            self.node_config
                .admission_control
                .need_to_check_mempool_before_validation,
        );
        let service = admission_control_grpc::create_admission_control(handle);

        let _ac_service_handle = spawn_service_thread(
            service,
            self.node_config.admission_control.address.clone(),
            self.node_config
                .admission_control
                .admission_control_service_port,
            "admission_control",
        );

        // Start Debug interface 启动debug接口
        let debug_service =
            node_debug_interface_grpc::create_node_debug_interface(NodeDebugService::new());
        let _debug_handle = spawn_service_thread(
            debug_service,
            self.node_config.admission_control.address.clone(),
            self.node_config
                .debug_interface
                .admission_control_node_debug_port,
            "debug_service",
        );

        info!(
            "Started AdmissionControl node on port {}",
            self.node_config
                .admission_control
                .admission_control_service_port
        );

        loop {
            thread::park();
        }
    }
}
