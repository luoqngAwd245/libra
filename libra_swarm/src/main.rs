// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use libra_swarm::{client, swarm::LibraSwarm};
use std::path::Path;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(
    name = "libra_swarm",
    author = "Libra",
    about = "Libra swarm to start local nodes"
)]
/// 集群启动参数，structopt是一个通过结构体来解析命令行参数。可以说它对clap库进行补充。
struct Args {
    /// Number of nodes to start (1 by default)
    /// 需要启动的节点数目（默认1）
    #[structopt(short = "n", long = "num_nodes")]
    pub num_nodes: Option<usize>,
    /// 不启用日志（为性能测试）
    /// Enable logging
    #[structopt(short = "l", long = "enable_logging")]
    pub enable_logging: bool,
    /// Start client 启动客户端
    #[structopt(short = "s", long = "start_client")]
    pub start_client: bool,
    /// Directory used by launch_swarm to output LibraNodes' config files, logs, libradb, etc,
    /// such that user can still inspect them after exit.
    /// If unspecified, a temporary dir will be used and auto deleted.
    ///  launch_swarm 直接用于输出libra节点的配置文件、日志和libradb的目录 等等。这样用户在退出后
    /// 仍然可以检查它们。如果没有指定， 临时目录会被使用并自动删除。
    #[structopt(short = "c", long = "config_dir")]
    pub config_dir: Option<String>,
    /// If specified, load faucet key from this file. Otherwise generate new keypair file.
    /// 如果已指定，请从此文件加faucet key。 否则生成新的密钥对文件。
    #[structopt(short = "f", long = "faucet_key_path")]
    pub faucet_key_path: Option<String>,
}
/// cargo run -p libra_swarm -- -s
/// 集群启动入口
fn main() {
    
    let args = Args::from_args(); //读取启动参数
    let num_nodes = args.num_nodes.unwrap_or(1); //获取节点数量
    // 获取fancet账户keypair，faucet key文件路径，临时文件路径
    let (faucet_account_keypair, faucet_key_file_path, _temp_dir) =
        generate_keypair::load_faucet_key_or_create_default(args.faucet_key_path);

    println!(
        "Faucet account created in (loaded from) file {:?}",
        faucet_key_file_path
    );
    // 启动集群
    let swarm = LibraSwarm::launch_swarm(
        num_nodes,
        !args.enable_logging,
        faucet_account_keypair,
        false, /* tee_logs */
        args.config_dir.clone(),
    );

    let config = &swarm.config.get_configs()[0].1; //获取节点配置
    let validator_set_file = &config.base.trusted_peers_file; //获取验证节点集合文件
    // 按下面提示在一个单独的进程中运行一个CLI客户端连接到你刚spawn的本地集群的节点
    println!("To run the Libra CLI client in a separate process and connect to the local cluster of nodes you just spawned, use this command:");
    println!(
        "\tcargo run --bin client -- -a localhost -p {} -s {:?} -m {:?}",
        config.admission_control.admission_control_service_port,
        swarm
            .dir
            .as_ref()
            .expect("fail to access output dir")
            .as_ref()
            .join(validator_set_file),
        faucet_key_file_path,
    );
    // 临时助记符文件
    let tmp_mnemonic_file = tempfile::NamedTempFile::new().unwrap();
    // 启动客户端
    if args.start_client {
        let client = client::InteractiveClient::new_with_inherit_io(
            *swarm.get_validators_public_ports().get(0).unwrap(),
            Path::new(&faucet_key_file_path),
            &tmp_mnemonic_file.into_temp_path(),
            swarm.get_trusted_peers_config_path(),
        );
        println!("Loading client...");
        let _output = client.output().expect("Failed to wait on child");
        println!("Exit client.");
    } else {
        // Explicitly capture CTRL-C to drop LibraSwarm.
        // 明确的捕获CTRL-C销毁集群
        let (tx, rx) = std::sync::mpsc::channel();
        ctrlc::set_handler(move || {
            tx.send(())
                .expect("failed to send unit when handling CTRL-C");
        })
        .expect("failed to set CTRL-C handler");
        println!("CTRL-C to exit.");
        rx.recv()
            .expect("failed to receive unit when handling CTRL-C");
    }
    if let Some(dir) = &args.config_dir {
        println!("Please manually cleanup {:?} after inspection", dir);
    }
    println!("Exit libra_swarm.");
}
