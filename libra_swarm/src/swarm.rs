// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    output_tee::{OutputTee, OutputTeeGuard},
    utils,
};
use config::config::NodeConfig;
use config_builder::swarm_config::{SwarmConfig, SwarmConfigBuilder};
use crypto::signing::KeyPair;
use debug_interface::NodeDebugClient;
use failure::prelude::*;
use logger::prelude::*;
use std::{
    collections::HashMap,
    env,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    str::FromStr,
};
use tempfile::TempDir;
use tools::output_capture::OutputCapture;

const LIBRA_NODE_BIN: &str = "libra_node";
/// 节点信息
pub struct LibraNode {
    node: Child, //子进程
    debug_client: NodeDebugClient, //节点调试客户端
    ac_port: u16, //AC端口
    peer_id: String, //对等节点ID
    log: PathBuf, //log目录
    output_tee_guard: Option<OutputTeeGuard>,
}

impl Drop for LibraNode {
    // When the LibraNode struct goes out of scope we need to kill the child process
    //当LibraNode结构出了作用域，我们需要终止子进程
    fn drop(&mut self) {
        // check if the process has already been terminated
        //检查进程是否已经终止
        match self.node.try_wait() {
            // The child process has already terminated, perhaps due to a crash
            //子进程已经终止，可能是由于崩溃
            Ok(Some(_)) => {}

            // The node is still running so we need to attempt to kill it
            // 节点正在运行我们试图终止它
            _ => {
                if let Err(e) = self.node.kill() {
                    panic!("LibraNode process could not be killed: '{}'", e);
                }
            }
        }
        if let Some(output_tee_guard) = self.output_tee_guard.take() {
            output_tee_guard.join();
        }
    }
}

impl LibraNode {
    /// 启动函数
    pub fn launch(
        config: &NodeConfig,
        config_path: &Path,
        logdir: &Path,
        disable_logging: bool,
        tee_logs: bool,
    ) -> Result<Self> {
        let peer_id = config.base.peer_id.clone();
        let log = logdir.join(format!("{}.log", peer_id));
        let log_file = File::create(&log)?;
        //节点启动命令参数
        let mut node_command = Command::new(utils::get_bin(LIBRA_NODE_BIN));
        // 构造参数
        node_command
            .current_dir(utils::workspace_root())
            .arg("-f")
            .arg(config_path)
            .args(&["-p", &peer_id]);
        if env::var("RUST_LOG").is_err() {
            // Only set our RUST_LOG if its not present in environment
            // 只有在环境中不存在时才设置我们的RUST_LOG
            node_command.env("RUST_LOG", "debug");
        }
        if disable_logging {
            node_command.arg("-d");
        }

        if tee_logs {
            // 可以通过管道与 所代表的进程交互
            node_command.stdout(Stdio::piped()).stderr(Stdio::piped());
        } else {
            node_command
                .stdout(log_file.try_clone()?)
                .stderr(log_file.try_clone()?);
        };
        // 启动节点进程，调用libra_node可执行程序启动子进程
        let mut node = node_command
            .spawn()
            .context("Error launching node process")?;

        let output_tee_guard = if tee_logs {
            let prefix = format!("[{}] ", &peer_id[..8]);
            let capture = OutputCapture::grab();
            let tee = OutputTee::new(
                capture,
                Box::new(log_file),
                Box::new(node.stdout.take().expect("Can't get child stdout")),
                Box::new(node.stderr.take().expect("Can't get child stderr")),
                prefix,
            );
            // 启动日志线程
            Some(tee.start())
        } else {
            None
        };
        // 启动调试客户端
        let debug_client = NodeDebugClient::new(
            "localhost",
            config.debug_interface.admission_control_node_debug_port,
        );
        Ok(Self {
            node,
            debug_client,
            ac_port: config.admission_control.admission_control_service_port,
            peer_id,
            log,
            output_tee_guard,
        })
    }

    pub fn peer_id(&self) -> String {
        self.peer_id.clone()
    }

    pub fn ac_port(&self) -> u16 {
        self.ac_port
    }

    pub fn get_log_contents(&self) -> Result<String> {
        let mut log = File::open(&self.log)?;
        let mut contents = String::new();
        log.read_to_string(&mut contents)?;

        Ok(contents)
    }

    fn get_metric(&self, metric_name: &str) -> Option<i64> {
        match self.debug_client.get_node_metric(metric_name) {
            Err(e) => {
                debug!(
                    "error getting {} for node: {}; error: {}",
                    metric_name, self.peer_id, e
                );
                None
            }
            Ok(maybeval) => {
                if maybeval.is_none() {
                    debug!("Node: {} did not report {}", self.peer_id, metric_name);
                }
                maybeval
            }
        }
    }

    pub fn check_connectivity(&self, expected_peers: i64) -> bool {
        if let Some(num_connected_peers) = self.get_metric("network_gauge{op=connected_peers}") {
            if num_connected_peers != expected_peers {
                debug!(
                    "Node '{}' Expected peers: {}, found peers: {}",
                    self.peer_id, expected_peers, num_connected_peers
                );
                return false;
            } else {
                return true;
            }
        }
        false
    }
    // 节点健康检查
    pub fn health_check(&mut self) -> HealthStatus {
        debug!("Health check on node '{}'", self.peer_id);

        // check if the process has terminated
        // 检查进程是否中止
        match self.node.try_wait() {
            // This would mean the child process has crashed
            //  这种情况意味着子进程已经crashed
            Ok(Some(status)) => {
                debug!("Node '{}' crashed with: {}", self.peer_id, status);
                return HealthStatus::Crashed(status);
            }

            // This is the case where the node is still running
            // 这种情况意味着子进程仍在运行
            Ok(None) => {}

            // Some other unknown error 未知类型错误
            Err(e) => {
                panic!("error attempting to query Node: {}", e);
            }
        }
        //查询节点度量标准
        match self.debug_client.get_node_metrics() {
            Ok(_) => {
                debug!("Node '{}' is healthy", self.peer_id);
                HealthStatus::Healthy
            }
            Err(e) => {
                debug!("Error querying metrics for node '{}'", self.peer_id);
                HealthStatus::RpcFailure(e)
            }
        }
    }
}

///健康状态枚举
pub enum HealthStatus {
    Healthy,
    Crashed(::std::process::ExitStatus),
    RpcFailure(failure::Error),
}

/// A wrapper that unifies PathBuf and TempDir.
/// 一个统一PathBuf和TempDir的包装器。
#[derive(Debug)]
pub enum LibraSwarmDir {
    Persistent(PathBuf),
    Temporary(TempDir),
}

impl AsRef<Path> for LibraSwarmDir {
    fn as_ref(&self) -> &Path {
        match self {
            LibraSwarmDir::Persistent(path_buf) => path_buf.as_path(),
            LibraSwarmDir::Temporary(temp_dir) => temp_dir.path(),
        }
    }
}

/// Struct holding instances and information of Libra Swarm
/// 结构控制Libra Swarm的实例和信息
pub struct LibraSwarm {
    // Output log, LibraNodes' config file, libradb etc, into this dir.
    //输出日志，LibraNodes的配置文件，libradb等，进入这个目录。
    pub dir: Option<LibraSwarmDir>,
    // Maps the peer id of a node to the LibraNode struct
    //将节点的peer id映射到LibraNode结构
    pub nodes: HashMap<String, LibraNode>,
    pub config: SwarmConfig,
    tee_logs: bool,
}
///Swarm启动错误枚举
#[derive(Debug, Fail)]
pub enum SwarmLaunchFailure {
    /// Timeout while waiting for nodes to start
    /// 等待节点启动超时
    #[fail(display = "Node launch check timeout")]
    LaunchTimeout,
    /// 节点返回状态表示崩溃
    /// Node return status indicates a crash
    #[fail(display = "Node crash")]
    NodeCrash,
    /// Timeout while waiting for the nodes to report that they're all interconnected
    /// 等待节点报告它们全部互连时超时
    #[fail(display = "Node connectivity check timeout")]
    ConnectivityTimeout,
}

impl LibraSwarm {
    /// 启动Swarm
    pub fn launch_swarm(
        num_nodes: usize,
        disable_logging: bool,
        faucet_account_keypair: KeyPair,
        tee_logs: bool,
        config_dir: Option<String>,
    ) -> Self {
        let num_launch_attempts = 5; //尝试5次
        for i in 0..num_launch_attempts {
            info!("Launch swarm attempt: {} of {}", i, num_launch_attempts);
            // 尝试启动
            match Self::launch_swarm_attempt(
                num_nodes,
                disable_logging,
                faucet_account_keypair.clone(),
                tee_logs,
                &config_dir,
            ) {
                Ok(swarm) => {
                    return swarm;
                }
                Err(e) => error!("Error launching swarm: {}", e),
            }
        }
        panic!("Max out {} attempts to launch swarm", num_launch_attempts);
    }
/// 尝试启动swarm
    fn launch_swarm_attempt(
        num_nodes: usize,
        disable_logging: bool,
        faucet_account_keypair: KeyPair,
        tee_logs: bool,
        config_dir: &Option<String>,
    ) -> std::result::Result<Self, SwarmLaunchFailure> {
        let dir = match config_dir {
            Some(dir_str) => {
                std::fs::create_dir_all(dir_str).expect("unable to create config dir");
                LibraSwarmDir::Persistent(
                    PathBuf::from_str(&dir_str).expect("unable to create config dir"),
                )
            }
            None => LibraSwarmDir::Temporary(
                tempfile::tempdir().expect("unable to create temporary config dir"),
            ),
        };
        let logs_dir_path = &dir.as_ref().join("logs");
        println!("Base directory containing logs and configs: {:?}", &dir);
        std::fs::create_dir(&logs_dir_path).unwrap();
        let base = utils::workspace_root().join("config/data/configs/node.config.toml");
        let mut config_builder = SwarmConfigBuilder::new();
        config_builder
            .with_ipv4()
            .with_nodes(num_nodes)
            .with_base(base)
            .with_output_dir(&dir)
            .with_faucet_keypair(faucet_account_keypair)
            .randomize_ports();
       // 构造配置
        let config = config_builder.build().unwrap();

        let mut swarm = Self {
            dir: Some(dir),
            nodes: HashMap::new(),
            config,
            tee_logs,
        };
        // For each config launch a node  为每个配置启动一个节点
        for (path, node_config) in swarm.config.get_configs() {
            let node = LibraNode::launch(
                &node_config,
                &path,
                &logs_dir_path,
                disable_logging,
                tee_logs,
            )
            .unwrap();
            swarm.nodes.insert(node.peer_id(), node);
        }
        //等待启动
        swarm.wait_for_startup()?;
        //等待连接
        swarm.wait_for_connectivity()?;

        info!("Successfully launched Swarm");

        Ok(swarm)
    }

    fn wait_for_connectivity(&self) -> std::result::Result<(), SwarmLaunchFailure> {
        // Early return if we're only launching a single node
        // 如果我们只启动单个节点，请尽早返回
        if self.nodes.len() == 1 {
            return Ok(());
        }

        let num_attempts = 60;

        for i in 0..num_attempts {
            debug!("Wait for connectivity attempt: {}", i);

            if self
                .nodes
                .values()
                .all(|node| node.check_connectivity(self.nodes.len() as i64 - 1))
            {
                return Ok(());
            }

            ::std::thread::sleep(::std::time::Duration::from_millis(1000));
        }

        Err(SwarmLaunchFailure::ConnectivityTimeout)
    }

    fn wait_for_startup(&mut self) -> std::result::Result<(), SwarmLaunchFailure> {
        let num_attempts = 120;
        let mut done = vec![false; self.nodes.len()];

        for i in 0..num_attempts {
            debug!("Wait for startup attempt: {} of {}", i, num_attempts);
            for (node, done) in self.nodes.values_mut().zip(done.iter_mut()) {
                if *done {
                    continue;
                }

                match node.health_check() {
                    HealthStatus::Healthy => *done = true,
                    HealthStatus::RpcFailure(_) => continue,
                    HealthStatus::Crashed(status) => {
                        error!(
                            "Libra node '{}' has crashed with status '{}'. Log output: '''{}'''",
                            node.peer_id,
                            status,
                            node.get_log_contents().unwrap()
                        );
                        return Err(SwarmLaunchFailure::NodeCrash);
                    }
                }
            }

            // Check if all the nodes have been successfully launched
            //检查是否所有节点都成功启动
            if done.iter().all(|status| *status) {
                return Ok(());
            }

            ::std::thread::sleep(::std::time::Duration::from_millis(1000));
        }

        Err(SwarmLaunchFailure::LaunchTimeout)
    }

    /// This function first checks the last committed round of all the nodes, picks the max
    /// value and then waits for all the nodes to catch up to that round.
    /// Once done, we can guarantee that all the txns committed before the invocation of this
    /// function are now available at all the nodes.
    /// 此函数首先检查所有节点的最后一个提交轮次，选择最大值，然后等待所有节点赶上该轮次。
    ///  完成后，我们可以保证在调用之前提交的所有txns  函数现在可在所有节点上使用。
    pub fn wait_for_all_nodes_to_catchup(&mut self) -> bool {
        let num_attempts = 60;
        let last_committed_round_str = "consensus{op=committed_blocks_count}";
        let mut done = vec![false; self.nodes.len()];

        let mut last_committed_round = 0;
        // First, try to retrieve the max value across all the committed rounds
        // 首先，尝试检索所有已提交轮次的最大值
        debug!("Calculating max committed round across the validators.");
        for node in self.nodes.values() {
            match node.get_metric(last_committed_round_str) {
                Some(val) => {
                    debug!("\tNode {} last committed round = {}", node.peer_id, val);
                    last_committed_round = last_committed_round.max(val);
                }
                None => {
                    debug!(
                        "\tNode {} last committed round unknown, assuming 0.",
                        node.peer_id
                    );
                }
            }
        }

        // Now wait for all the nodes to catch up to the max.
        // 等待所有节点赶上最大轮次
        for i in 0..num_attempts {
            debug!(
                "Wait for catchup, target_commit_round = {}, attempt: {} of {}",
                last_committed_round, i, num_attempts
            );
            for (node, done) in self.nodes.values_mut().zip(done.iter_mut()) {
                if *done {
                    continue;
                }

                match node.get_metric(last_committed_round_str) {
                    Some(val) => {
                        if val >= last_committed_round {
                            debug!(
                                "\tNode {} is caught up with last committed round {}",
                                node.peer_id, val
                            );
                            *done = true;
                        } else {
                            debug!(
                                "\tNode {} is not caught up yet with last committed round {}",
                                node.peer_id, val
                            );
                        }
                    }
                    None => {
                        debug!(
                            "\tNode {} last committed round unknown, assuming 0.",
                            node.peer_id
                        );
                    }
                }
            }

            // Check if all the nodes have been successfully caught up
            // 检查是否所有节点都已成功捕获
            if done.iter().all(|status| *status) {
                return true;
            }

            ::std::thread::sleep(::std::time::Duration::from_millis(1000));
        }

        false
    }

    /// Vector with the public AC ports of the validators.
    /// 拥有验证节点公开AC端口的Vector
    pub fn get_validators_public_ports(&self) -> Vec<u16> {
        self.nodes.values().map(|node| node.ac_port()).collect()
    }

    /// Vector with the peer ids of the validators in the swarm.
    /// 拥有群中验证器的对等ID的Vector。
    pub fn get_validators_ids(&self) -> Vec<String> {
        self.nodes.keys().cloned().collect()
    }

    /// Vector with the debug ports of all the validators in the swarm.
    /// 拥有群中所有验证器的调试端口进行的Vector
    pub fn get_validators_debug_ports(&self) -> Vec<u16> {
        self.config
            .get_configs()
            .iter()
            .map(|(_, c)| c.debug_interface.admission_control_node_debug_port)
            .collect()
    }

    pub fn get_validator(&self, peer_id: &str) -> Option<&LibraNode> {
        self.nodes.get(peer_id)
    }

    pub fn kill_node(&mut self, peer_id: &str) {
        self.nodes.remove(peer_id);
    }

    pub fn add_node(
        &mut self,
        peer_id: String,
        disable_logging: bool,
    ) -> std::result::Result<(), SwarmLaunchFailure> {
        // First take the configs out to not keep immutable borrow on self when calling
        // `launch_node`.
        // 首先取出配置，以便在调用`launch_node`时不要让自己不可靠的借用
        self.launch_node(peer_id, disable_logging)
    }

    fn launch_node(
        &mut self,
        peer_id: String,
        disable_logging: bool,
    ) -> std::result::Result<(), SwarmLaunchFailure> {
        let (path, config) = self
            .config
            .get_configs()
            .iter()
            .find(|(_path, config)| config.base.peer_id == peer_id)
            .expect(
                &format!(
                    "PeerId {} not found in any of the admission control service ports.",
                    peer_id
                )[..],
            );
        let logs_dir_path = self.dir.as_ref().map(|x| x.as_ref().join("logs")).unwrap();
        let mut node =
            LibraNode::launch(config, path, &logs_dir_path, disable_logging, self.tee_logs)
                .unwrap();
        for _ in 0..60 {
            if let HealthStatus::Healthy = node.health_check() {
                self.nodes.insert(peer_id, node);
                return self.wait_for_connectivity();
            }
            ::std::thread::sleep(::std::time::Duration::from_millis(1000));
        }
        Err(SwarmLaunchFailure::LaunchTimeout)
    }

    pub fn get_trusted_peers_config_path(&self) -> String {
        let (path, _) = self.config.get_trusted_peers_config();
        path.canonicalize()
            .expect("Unable to get canonical path of trusted peers config file")
            .to_str()
            .unwrap()
            .to_string()
    }
}

impl Drop for LibraSwarm {
    fn drop(&mut self) {
        // If panicking, we don't want to gc the swarm directory.
        // 如果panicking，我们不想gc swarm目录。
        if std::thread::panicking() {
            if let Some(dir) = self.dir.take() {
                if let LibraSwarmDir::Temporary(temp_dir) = dir {
                    let log_path = temp_dir.into_path();
                    println!("logs located at {:?}", log_path);
                }
            }
        }
    }
}
