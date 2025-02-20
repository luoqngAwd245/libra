// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use admission_control_proto::proto::{
    admission_control::{
        SubmitTransactionRequest, SubmitTransactionResponse as ProtoSubmitTransactionResponse,
    },
    admission_control_grpc::AdmissionControlClient,
};
use client::{AccountData, AccountStatus};
use crypto::signing::KeyPair;
use generate_keypair::load_key_from_file;
use lazy_static::lazy_static;
use logger::prelude::*;
use metrics::OpMetrics;
use rand::Rng;
use std::{collections::HashMap, convert::TryInto, sync::Arc, thread, time};
use types::{account_address::AccountAddress, account_config::association_address};

pub mod grpc_helpers;
pub mod ruben_opt;
pub mod txn_generator;

use grpc_helpers::{
    divide_items, get_account_states, submit_and_wait_txn_requests, sync_account_sequence_number,
};
use txn_generator::gen_mint_txn_requests;

lazy_static! {
    pub static ref OP_COUNTER: OpMetrics = OpMetrics::new_and_registered("benchmark");
}

/// Benchmark library for Libra Blockchain. Benchmark 模块for libra 区块链
///
/// Benchmarker aims to automate the process of submitting TXNs to admission control
/// in a configurable mechanism, and wait for accepted TXNs comitted or timeout.
/// Current main usage is measuring TXN throughput.
/// How to run a benchmarker (see RuBen in bin/ruben.rs):
/// 1. Create a benchmarker with AdmissionControlClient(s) and NodeDebugClient,
/// 2. Generate accounts (using txn_generator module) and mint them: mint_accounts.
/// 3. Play transactions offline-generated by txn_generator module:
///    submit_and_wait_txn_committed, measure_txn_throughput.
/// Metrics reported include:
/// * Counters related to:
///   * TXN generation: requested_txns, created_txns, sign_failed_txns;
///   * Submission to AC and AC response: submit_txns.{ac_status_code},
///     submit_txns.{mempool_status_code}, submit_txns.{vm_status}, submit_txns.{grpc_error};
///   * Final status within epoch: committed_txns, timedout_txns;
/// * Gauges: request_duration_ms, running_duration_ms, request_throughput, txns_throughput.
/// Benchmarker旨在以可配置的机制自动化将TXN提交到准入控制的过程，并等待已接受的TXN提交或超时。
///  当前的主要用途是测量TXN吞吐量。
///如何运行基准测试程序（请参阅bin / ruben.rs中的RuBen）：
///1.使用AdmissionControlClient（s）和NodeDebugClient创建基准测试程序，
///2.生成帐户（使用txn_generator模块）并铸造它们：mint_accounts。
///3.播放由txn_generator模块离线生成的事务：
///  commit_and_wait_txn_committed，measure_txn_throughput。
///报告的指标包括：
///  *计数器涉及：
///* TXN生成：requested_txns，created_txns，sign_failed_txns；
///  *提交给AC和AC响应：submit_txns。{ac_status_code}，submit_txns。{mempool_status_code}，submit_txns
/// 。{vm_status}，submit_txns。{grpc_error}；
///*时期内的最终状态：committed_txns，timedout_txns；
///*量规：request_duration_ms，running_duration_ms，request_throughput，txns_throughput。
pub struct Benchmarker {
    /// Using multiple clients can help improve the request speed.
    /// 使用多个客户端可以帮助提高请求速度。
    clients: Vec<Arc<AdmissionControlClient>>,
    /// Upper bound duration to stagger the clients before submitting TXNs.
    /// 上限持续时间，用于在提交TXN之前错开客户端。
    stagger_range_ms: u16,
    /// Persisted sequence numbers for generated accounts and faucet account
    /// BEFORE playing new round of TXNs.
    /// 在播放新一轮的TXN之前，已生成帐户和水龙头帐户的永久序列号。
    prev_sequence_numbers: HashMap<AccountAddress, u64>,
}

impl Benchmarker {
    /// Construct Benchmarker with a vector of AC clients and a NodeDebugClient.
    /// 使用AC客户端和NodeDebugClient的向量构造Benchmarker。
    pub fn new(clients: Vec<AdmissionControlClient>, stagger_range_ms: u16) -> Self {
        if clients.is_empty() {
            panic!("failed to create benchmarker without any AdmissionControlClient");
        }
        let arc_clients = clients.into_iter().map(Arc::new).collect();
        let prev_sequence_numbers = HashMap::new();
        Benchmarker {
            clients: arc_clients,
            stagger_range_ms,
            prev_sequence_numbers,
        }
    }

    /// -------------------------------------------------------------------- ///
    ///  Benchmark setup: Load faucet account and minting APIs and helpers.  ///
    /// 基准设置：加载水龙头帐户并创建API和助手。                            ///
    /// -------------------------------------------------------------------- ///

    /// Load keypair from given faucet_account_path,
    /// then try to sync with a validator to get up-to-date faucet account's sequence number.
    /// Why restore faucet account: Benchmarker as a client can be stopped/restarted repeatedly
    /// while the libra swarm as a server keeping running.
    /// 从给定的faucet_account_path加载密钥对，然后尝试与验证器同步以获取最新的水龙头帐户的序列号。
    /// 为什么要恢复水龙头帐户：作为天秤座的客户端可以反复停止/重新启动，而作为服务器的天秤座蜂拥而至。
    fn load_faucet_account(&self, faucet_account_path: &str) -> AccountData {
        let faucet_account_keypair: KeyPair =
            load_key_from_file(faucet_account_path).expect("invalid faucet keypair file");
        let address = association_address();
        // Request and wait for account's (sequence_number, account_status) from a validator.
        // Assume Benchmarker is the ONLY active client in the libra network.
        // 请求并等待验证者提供的帐户（sequence_number，account_status）。 假设Benchmarker是天秤座
        // 网络中唯一的活动客户端。
        let client = self
            .clients
            .get(0)
            .expect("no available AdmissionControlClient");
        let states = get_account_states(client, &[address]);
        let (sequence_number, status) = states
            .get(&address)
            .expect("failed to get faucet account from validator");
        assert_eq!(status, &AccountStatus::Persisted);
        AccountData {
            address,
            key_pair: Some(faucet_account_keypair),
            sequence_number: *sequence_number,
            status: status.clone(),
        }
    }

    /// Minting given accounts using self's AC client(s).
    /// Mint TXNs must be 100% successful in order to continue benchmark.
    /// Therefore mint_accounts() will panic when any mint TXN is not accepted or fails.
    /// Known issue: Minting opereations from two different Benchmarker instances
    /// will fail because they are sharing the same faucet account.
    /// 使用自己的AC客户铸造给定的帐户。
    ///薄荷TXN必须100％成功才能继续进行基准测试。
    ///因此，当任何薄荷TXN不被接受或失败时，mint_accounts（）将会恐慌。
    ///  已知问题：由于两个共享Benchmarker实例共享同一个水龙头帐户，因此无法进行铸币操作。
    pub fn mint_accounts(&mut self, mint_key_file_path: &str, accounts: &[AccountData]) {
        let mut faucet_account = self.load_faucet_account(mint_key_file_path);
        self.prev_sequence_numbers
            .insert(faucet_account.address, faucet_account.sequence_number);
        for account in accounts.iter() {
            self.prev_sequence_numbers
                .insert(account.address, account.sequence_number);
        }

        let mint_requests = gen_mint_txn_requests(&mut faucet_account, accounts);
        // Disable client staggering for mint operations.
        // 禁用客户端交错操作以进行薄荷糖操作。
        let stagger_range_ms = self.stagger_range_ms;
        self.stagger_range_ms = 1;
        let (num_accepted, num_committed, _, _) =
            self.submit_and_wait_txn_committed(&mint_requests, &mut [faucet_account]);
        self.stagger_range_ms = stagger_range_ms;
        // We stop immediately if any minting fails.
        // 如果铸造失败，我们将立即停止。
        if num_accepted != mint_requests.len() || num_accepted - num_committed > 0 {
            panic!(
                "{} of {} mint transaction(s) accepted, and {} failed",
                num_accepted,
                mint_requests.len(),
                num_accepted - num_committed,
            )
        }
    }

    /// ----------------------------------------------------------------- ///
    ///  Transaction submission and waiting for commit APIs and helpers.  ///
    /// 事务提交以及等待提交的API和助手。                                 ///
    /// ----------------------------------------------------------------- ///

    /// Put client to sleep for a random duration before submitting TXN requests.
    /// Return how long the client is scheduled to be delayed.
    /// 在提交TXN请求之前，让客户端随机睡眠。 返回客户端计划延迟的时间。
    fn stagger_client(stagger_range_ms: u16) -> u16 {
        let mut rng = rand::thread_rng();
        // Double check the upper bound value to be no less than 1.
        // 仔细检查上限值不小于1。
        let duration = rng.gen_range(0, std::cmp::max(1, stagger_range_ms));
        thread::sleep(time::Duration::from_millis(u64::from(duration)));
        duration
    }

    /// Send requests to AC async, wait for responses from AC.
    /// Return #accepted TXNs and submission duration.
    /// 发送请求到AC异步，等待来自AC的响应。
    ///返回＃接受的TXN和提交时间。
    pub fn submit_txns(&mut self, txn_reqs: &[SubmitTransactionRequest]) -> (usize, u128) {
        let txn_req_chunks = divide_items(txn_reqs, self.clients.len());
        let now = time::Instant::now();
        // Zip txn_req_chunks with clients: when first iter returns none,
        // zip will short-circuit and next will not be called on the second iter.
        // 与客户端一起压缩txn_req_chunks：当第一个迭代未返回任何内容时，zip将会短路，而在第二个迭代中不会调用下一步。
        let children: Vec<thread::JoinHandle<_>> = txn_req_chunks
            .zip(self.clients.iter().cycle())
            .map(|(chunk, client)| {
                let local_chunk = Vec::from(chunk);
                let local_client = Arc::clone(client);
                let stagger_range_ms = self.stagger_range_ms;
                // Spawn threads with corresponding client.
                // 生成具有相应客户端的线程。
                thread::spawn(
                    // Dispatch TXN requests to client and submit, return the list of responses
                    // that are accepted by AC, and how long the client is delayed.
                    // 将TXN请求分派给客户端并提交，返回AC接受的响应列表以及客户端延迟多长时间。
                    move || -> (Vec<ProtoSubmitTransactionResponse>, u16) {
                        let delay_duration_ms = Self::stagger_client(stagger_range_ms);
                        info!(
                            "Dispatch a chunk of {} requests to client and start to submit after staggered {} ms.",
                            local_chunk.len(),
                            delay_duration_ms,
                        );
                        (submit_and_wait_txn_requests(&local_client, &local_chunk), delay_duration_ms)
                    },
                )
            })
            .collect();
        // Wait for threads and gather reponses. 等待线程并收集响应。
        // TODO: Group response by error type and report staticstics.
        // TODO：按错误类型和报告统计信息对响应进行分组。
        let mut txn_resps: Vec<ProtoSubmitTransactionResponse> = vec![];
        let mut delay_duration_ms = self.stagger_range_ms;
        for child in children {
            let resp_tuple = child.join().expect("failed to join a request thread");
            txn_resps.extend(resp_tuple.0.into_iter());
            // Start counting time as soon as the first client starts to submit TXNs.
            // 第一个客户开始提交TXN后，立即开始计时。
            delay_duration_ms = std::cmp::min(delay_duration_ms, resp_tuple.1);
        }
        let mut request_duration_ms = now.elapsed().as_millis();
        // Calling stagger_client() should ensure delay duration strictly < self.stagger_range_ms.
        if delay_duration_ms < self.stagger_range_ms {
            request_duration_ms -= u128::from(delay_duration_ms);
        }
        info!(
            "Submitted and accepted {} TXNs within {} ms.",
            txn_resps.len(),
            request_duration_ms,
        );
        (txn_resps.len(), request_duration_ms)
    }

    /// Wait for accepted TXNs to commit or time out: for any account, if its sequence number
    /// (bumpped during TXN generation) equals the one synchronized from validator,
    /// denoted as sync sequence number, then all its TXNs are committed.
    /// Return senders' most up-to-date sync sequence numbers and how long we have waited.
    /// 等待接受的TXN提交或超时：对于任何帐户，如果其序列号（在TXN生成过程中发生碰撞）等于从验证程序
    /// 序列号（表示为同步序列号），则将提交其所有TXN。 返回发件人的最新同步序列号以及我们等待了多长时间。
    pub fn wait_txns(&self, senders: &[AccountData]) -> (HashMap<AccountAddress, u64>, u128) {
        let account_chunks = divide_items(senders, self.clients.len());
        let now = time::Instant::now();
        let children: Vec<thread::JoinHandle<HashMap<_, _>>> = account_chunks
            .zip(self.clients.iter().cycle())
            .map(|(chunk, client)| {
                let local_chunk = Vec::from(chunk);
                let local_client = Arc::clone(client);
                info!(
                    "Dispatch a chunk of {} accounts to client.",
                    local_chunk.len()
                );
                thread::spawn(move || -> HashMap<AccountAddress, u64> {
                    sync_account_sequence_number(&local_client, &local_chunk)
                })
            })
            .collect();
        let mut sequence_numbers: HashMap<AccountAddress, u64> = HashMap::new();
        for child in children {
            let sequence_number_chunk = child.join().expect("failed to join a wait thread");
            sequence_numbers.extend(sequence_number_chunk);
        }
        let wait_duration_ms = now.elapsed().as_millis();
        info!("Waited for TXNs for {} ms", wait_duration_ms);
        (sequence_numbers, wait_duration_ms)
    }

    /// -------------------------------------------------- ///
    ///  Transaction playing, throughput measureing APIs.  ///
    /// 事务处理，吞吐量测量API。
    /// -------------------------------------------------- ///

    /// With the previous stored sequence number (e.g. self.prev_sequence_numbers)
    /// and the synchronized sequence number from validator, calculate how many TXNs are committed.
    /// Update both senders sequence numbers and self.prev_sequence_numbers to the just-queried
    /// synchrnized sequence numbers. Return (#committed, #uncommitted) TXNs.
    /// Reason to backtrace sender's sequence number:
    /// If some of sender's TXNs are not committed because they are rejected by AC,
    /// we should use the synchronized sequence number in future TXN generation.
    /// On the other hand, if sender's TXNs are accepted but just waiting to be committed,
    /// part of the newly generated TXNs will be rejected by AC due to old sequence number,
    /// but eventually local account's sequence number will be new enough to get accepted.
    /// 使用先前存储的序列号（例如self.prev_sequence_numbers）和来自验证程序的同步序列号，计算提交了多少TXN。
    ///将发送方序列号和self.prev_sequence_numbers更新为刚查询的同步序列号。 返回（＃committed，＃uncommitted）TXN。
    ///回溯发件人序列号的原因：
    ///如果某些发件人的TXN由于被AC拒绝而未提交，则我们应该在以后的TXN生成中使用同步的序列号。
    /// 另一方面，如果发件人的TXN被接受但只是等待提交，则新生成的TXN的一部分将由于旧序列号而被AC拒绝，
    /// 但是最终本地帐户的序列号将是足够新的以被接受。
    fn check_txn_results(
        &mut self,
        senders: &mut [AccountData],
        sync_sequence_numbers: &HashMap<AccountAddress, u64>,
    ) -> (usize, usize) {
        let mut committed_txns = 0;
        let mut uncommitted_txns = 0;
        // Invariant for any account X in Benchmarker:
        // 1) X's current persisted sequence number (X.sequence_number) >=
        //    X's synchronized sequence number (sync_sequence_number[X])
        // 2) X's current persisted sequence number (X.sequence_number) >=
        //    X's previous persisted sequence number (self.prev_sequence_numbers[X])
        //  Benchmarker中的任何帐户X不变：
        //  1）X的当前持久序列号（X.sequence_number）> =
        // X的同步序号（sync_sequence_number [X]）
        // 2）X的当前持久序列号（X.sequence_number）> =
        //  X的先前的保留序列号（self.prev_sequence_numbers [X]）
        for sender in senders.iter_mut() {
            let prev_sequence_number = self
                .prev_sequence_numbers
                .get_mut(&sender.address)
                .expect("Sender doesn't exist in Benchmark environment");
            let sync_sequence_number = sync_sequence_numbers
                .get(&sender.address)
                .expect("Sender doesn't exist in validators");
            assert!(sender.sequence_number >= *sync_sequence_number);
            assert!(*sync_sequence_number >= *prev_sequence_number);
            if sender.sequence_number > *sync_sequence_number {
                error!("Account {:?} has uncommitted TXNs", sender.address);
            }
            committed_txns += *sync_sequence_number - *prev_sequence_number;
            uncommitted_txns += sender.sequence_number - *sync_sequence_number;
            *prev_sequence_number = *sync_sequence_number;
            sender.sequence_number = *sync_sequence_number;
        }
        info!(
            "#committed TXNs = {}, #uncommitted TXNs = {}",
            committed_txns, uncommitted_txns
        );
        let committed_txns_usize = committed_txns
            .try_into()
            .expect("Unable to convert u64 to usize");
        let uncommitted_txns_usize = uncommitted_txns
            .try_into()
            .expect("Unable to convert u64 to usize");
        OP_COUNTER.inc_by("committed_txns", committed_txns_usize);
        OP_COUNTER.inc_by("timedout_txns", uncommitted_txns_usize);
        (committed_txns_usize, uncommitted_txns_usize)
    }

    /// Implement the general way to submit TXNs to Libra and then
    /// wait for all accepted ones to become committed.
    /// Return (#accepted TXNs, #committed TXNs, submit duration, wait duration).
    /// 实现将TXN提交给Libra的一般方法，然后等待所有接受的承诺成为现实。 返回（＃接受的TXN，＃提交的TXN，
    /// 提交持续时间，等待时间）。
    pub fn submit_and_wait_txn_committed(
        &mut self,
        txn_reqs: &[SubmitTransactionRequest],
        senders: &mut [AccountData],
    ) -> (usize, usize, u128, u128) {
        let (num_txns_accepted, submit_duration_ms) = self.submit_txns(txn_reqs);
        let (sync_sequence_numbers, wait_duration_ms) = self.wait_txns(senders);
        let (num_committed, _) = self.check_txn_results(senders, &sync_sequence_numbers);
        (
            num_txns_accepted,
            num_committed,
            submit_duration_ms,
            wait_duration_ms,
        )
    }

    /// Calcuate average committed transactions per second.
    /// 计算每秒的平均承诺事务数。
    fn calculate_throughput(num_txns: usize, duration_ms: u128, prefix: &str) -> f64 {
        assert!(duration_ms > 0);
        let throughput = num_txns as f64 * 1000f64 / duration_ms as f64;
        info!(
            "{} throughput est = {} txns / {} ms = {:.2} rps.",
            prefix, num_txns, duration_ms, throughput,
        );
        throughput
    }

    /// Similar to submit_and_wait_txn_committed but with timing.
    /// How given TXNs are played and how time durations (submission, commit and running)
    /// are defined are illustrated as follows:
    ///                t_submit                AC responds all requests
    /// |==============================================>|
    ///                t_commit (unable to measure)     Storage stores all committed TXNs
    ///    |========================================================>|
    ///                t_run                            1 epoch of measuring finishes.
    /// |===========================================================>|
    /// Estimated TXN throughput from user perspective = #TXN / t_run.
    /// Estimated request throughput = #TXN / t_submit.
    /// Estimated TXN throughput internal to libra = #TXN / t_commit, not measured by this API.
    /// Return request througnhput and TXN throughput.
    /// 与commit_and_wait_txn_committed类似，但有时间限制。
    ///播放给定TXN的方式以及持续时间（提交，提交和运行）的定义方式如下所示：
    ///                 t_submit AC响应所有请求
    ///  | ============================================ ||
    ///                 t_commit（无法衡量）存储存储所有已提交的TXN
    ///============================================= ||
    ///                t_run 1个测量完成的时期。
    ///============================================= ||
    ///  从用户角度估计的TXN吞吐量= #TXN / t_run。
    ///  估算的请求吞吐量= #TXN / t_submit。
    ///  libra内部的估算TXN吞吐量= #TXN / t_commit，未通过此API测量。
    ///返回请求吞吐量和TXN吞吐量。
    pub fn measure_txn_throughput(
        &mut self,
        txn_reqs: &[SubmitTransactionRequest],
        senders: &mut [AccountData],
    ) -> (f64, f64) {
        let (_, num_committed, submit_duration_ms, wait_duration_ms) =
            self.submit_and_wait_txn_committed(txn_reqs, senders);
        let request_throughput =
            Self::calculate_throughput(txn_reqs.len(), submit_duration_ms, "REQ");
        let running_duration_ms = submit_duration_ms + wait_duration_ms;
        let txn_throughput = Self::calculate_throughput(num_committed, running_duration_ms, "TXN");

        OP_COUNTER.set("submit_duration_ms", submit_duration_ms as usize);
        OP_COUNTER.set("wait_duration_ms", wait_duration_ms as usize);
        OP_COUNTER.set("running_duration_ms", running_duration_ms as usize);
        OP_COUNTER.set("request_throughput", request_throughput as usize);
        OP_COUNTER.set("txn_throughput", txn_throughput as usize);

        (request_throughput, txn_throughput)
    }
}
