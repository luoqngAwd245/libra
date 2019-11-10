// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use admission_control_proto::proto::{
    admission_control::SubmitTransactionRequest, admission_control_grpc::AdmissionControlClient,
};
use benchmark::{
    ruben_opt::{Executable, Opt},
    txn_generator::{
        gen_accounts, gen_pairwise_transfer_txn_requests, gen_ring_transfer_txn_requests,
    },
    Benchmarker,
};
use client::AccountData;
use grpcio::{ChannelBuilder, EnvBuilder};
use libra_wallet::wallet_library::WalletLibrary;
use logger::{self, prelude::*};
use metrics::metric_server::start_server;
use std::sync::Arc;

/// Helper method to generate repeated TXNs from a function object, which is soon to be
/// replaced by generic struct that implements TxnGenerator trait.
/// 从函数对象生成重复TXN的Helper方法，该对象很快将被实现TxnGenerator特征的通用结构所取代。
#[allow(bare_trait_objects)]
fn gen_txn_load(
    txn_generator: &Fn(&WalletLibrary, &mut [AccountData]) -> Vec<SubmitTransactionRequest>,
    wallet: &WalletLibrary,
    accounts: &mut [AccountData],
    num_rounds: u64,
) -> Vec<SubmitTransactionRequest> {
    let mut repeated_tx_reqs = vec![];
    for _ in 0..num_rounds {
        let tx_reqs = txn_generator(wallet, accounts);
        repeated_tx_reqs.extend(tx_reqs.into_iter());
    }
    repeated_tx_reqs
}

/// Simply submit some TXNs to test the liveness of the network. Here we use ring TXN pattern
/// to generate request, which scales linear with the number of accounts.
/// In one epoch, we play the sequence of ring TXN requests repeatedly for num_rounds times.
/// 只需提交一些TXN即可测试网络的运行状况。 在这里，我们使用环形TXN模式来生成请求，该请求与帐户数量成线性比例。
///在一个纪元内，我们重复播放环形TXN请求的序列，持续num_rounds次。
fn test_liveness(
    bm: &mut Benchmarker,
    accounts: &mut [AccountData],
    wallet: WalletLibrary,
    num_rounds: u64,
    num_epochs: u64,
) {
    for _ in 0..num_epochs {
        let repeated_tx_reqs = gen_txn_load(
            &gen_ring_transfer_txn_requests,
            &wallet,
            accounts,
            num_rounds,
        );
        bm.submit_and_wait_txn_committed(&repeated_tx_reqs, accounts);
    }
}

/// Measure both `burst` and `epoch` throughput with pairwise TXN pattern.
/// * `burst throughput`: the average committed txns per second during one run of playing TXNs
///   (e.g., Benchmarker::measure_txn_throughput). Since time is counted from submission until all
///   TXNs are committed, this measurement is in a sense the user-side throughput. In one epoch, we
///   play the pairwise TXN request sequence repeatedly for num_rounds times.
/// * `epoch throughput`: Since single run of playing TXNs may have high variance, we can repeat
///   playing TXNs many times and calculate the averaged `burst throughput` along with standard
///   deviation (will be added shortly).
/// 用成对的TXN模式测量“突发”和“历时”吞吐量。
///*“突发吞吐量”：在运行TXN的一次运行中平均每秒提交的txns（例如enchmarker :: measure_txn_throughput）。
/// 由于从提交到所有TXN提交都需要时间，因此从某种意义上说，此度量是用户方吞吐量。 在一个时期中，我们重复
/// 执行成对TXN请求序列达num_rounds次。
///*'epoch吞吐率'：由于单次运行的TXN可能会有较大的差异，因此我们可以重复播放TXN多次，并计算平均的
///“突发吞吐率”和标准差（将很快添加）。
pub(crate) fn measure_throughput(
    bm: &mut Benchmarker,
    accounts: &mut [AccountData],
    wallet: WalletLibrary,
    num_rounds: u64,
    num_epochs: u64,
) {
    let mut txn_throughput_seq = vec![];
    for _ in 0..num_epochs {
        let repeated_tx_reqs = gen_txn_load(
            &gen_pairwise_transfer_txn_requests,
            &wallet,
            accounts,
            num_rounds,
        );
        let txn_throughput = bm.measure_txn_throughput(&repeated_tx_reqs, accounts);
        txn_throughput_seq.push(txn_throughput);
    }
    info!(
        "{:?} epoch(s) of TXN throughput = {:?}",
        num_epochs, txn_throughput_seq
    );
}

fn create_ac_client(conn_addr: &str) -> AdmissionControlClient {
    let env_builder = Arc::new(EnvBuilder::new().name_prefix("ac-grpc-").build());
    let ch = ChannelBuilder::new(env_builder).connect(&conn_addr);
    AdmissionControlClient::new(ch)
}

/// Creat a vector of AdmissionControlClient and connect them to validators.
fn create_ac_clients(
    num_clients: usize,
    validator_addresses: &[String],
) -> Vec<AdmissionControlClient> {
    let mut clients: Vec<AdmissionControlClient> = vec![];
    for i in 0..num_clients {
        let index = i % validator_addresses.len();
        let client = create_ac_client(&validator_addresses[index]);
        clients.push(client);
    }
    clients
}

pub(crate) fn create_benchmarker_from_opt(args: &Opt) -> Benchmarker {
    // Create AdmissionControlClient instances.
    let clients = create_ac_clients(args.num_clients, &args.validator_addresses);
    // Ready to instantiate Benchmarker.
    Benchmarker::new(clients, args.stagger_range_ms)
}

/// Benchmarker is not a long-lived job, so starting a server and expecting it to be polled
/// continuously is not ideal. Directly pushing metrics when benchmarker is running
/// can be achieved by using Pushgateway.
/// Benchmarker并不是一项长期的工作，因此启动服务器并期望对其进行连续轮询不是理想的选择。
/// 通过使用Pushgateway，可以在运行基准测试程序时直接推送指标。
fn try_start_metrics_server(args: &Opt) {
    if let Some(metrics_server_address) = &args.metrics_server_address {
        let address = metrics_server_address.clone();
        std::thread::spawn(move || {
            start_server(address);
        });
    }
}

fn main() {
    let _g = logger::set_default_global_logger(false, Some(256));
    info!("RuBen: the utility to (Ru)n (Ben)chmarker");
    let args = Opt::new_from_args();
    info!("Parsed arguments: {:#?}", args);

    try_start_metrics_server(&args);
    let mut bm = create_benchmarker_from_opt(&args);
    let mut wallet = WalletLibrary::new();
    let mut accounts = gen_accounts(&mut wallet, args.num_accounts);
    bm.mint_accounts(&args.faucet_key_file_path, &accounts);
    match args.executable {
        Executable::TestLiveness => {
            test_liveness(
                &mut bm,
                &mut accounts,
                wallet,
                args.num_rounds,
                args.num_epochs,
            );
        }
        Executable::MeasureThroughput => {
            measure_throughput(
                &mut bm,
                &mut accounts,
                wallet,
                args.num_rounds,
                args.num_epochs,
            );
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::{create_benchmarker_from_opt, measure_throughput};
    use benchmark::{
        ruben_opt::{Executable, Opt},
        txn_generator::gen_accounts,
        OP_COUNTER,
    };
    use libra_swarm::swarm::LibraSwarm;
    use libra_wallet::wallet_library::WalletLibrary;
    use rusty_fork::{rusty_fork_id, rusty_fork_test, rusty_fork_test_name};

    rusty_fork_test! {
        #[test]
        fn test_benchmarker_counters() {
            let (faucet_account_keypair, faucet_key_file_path, _temp_dir) =
                generate_keypair::load_faucet_key_or_create_default(None);
            let swarm = LibraSwarm::launch_swarm(
                4,      /* num_nodes */
                true,   /* disable_logging */
                faucet_account_keypair,
                false,  /* tee_logs */
                None,   /* config_dir */
            );
            let mut args = Opt {
                validator_addresses: Vec::new(),
                debug_address: None,
                swarm_config_dir: Some(String::from(
                    swarm.dir.as_ref().unwrap().as_ref().to_str().unwrap(),
                )),
                // Don't start metrics server as we are not testing with prometheus.
                metrics_server_address: None,
                faucet_key_file_path,
                num_accounts: 4,
                free_lunch: 10_000_000,
                num_clients: 4,
                stagger_range_ms: 1,
                num_rounds: 4,
                num_epochs: 2,
                executable: Executable::MeasureThroughput,
            };
            args.try_parse_validator_addresses();
            let mut bm = create_benchmarker_from_opt(&args);
            let mut wallet = WalletLibrary::new();
            let mut accounts = gen_accounts(&mut wallet, args.num_accounts);
            bm.mint_accounts(&args.faucet_key_file_path, &accounts);
            measure_throughput(&mut bm, &mut accounts, wallet, args.num_rounds, args.num_epochs);
            let requested_txns = OP_COUNTER.counter("requested_txns").get();
            let created_txns = OP_COUNTER.counter("created_txns").get();
            let sign_failed_txns = OP_COUNTER.counter("sign_failed_txns").get();
            assert_eq!(requested_txns, created_txns + sign_failed_txns);
            let accepted_txns = OP_COUNTER.counter("submit_txns.Accepted").get();
            let committed_txns = OP_COUNTER.counter("committed_txns").get();
            let timedout_txns = OP_COUNTER.counter("timedout_txns").get();
            // Why `<=`: timedout TXNs in previous epochs can be committed in the next epoch.
            assert!(accepted_txns <= committed_txns + timedout_txns);
        }
    }
}
