// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{commands::*, grpc_client::GRPCClient, AccountData, AccountStatus};
use admission_control_proto::proto::admission_control::SubmitTransactionRequest;
use config::trusted_peers::TrustedPeersConfig;
use crypto::signing::KeyPair;
use failure::prelude::*;
use futures::{future::Future, stream::Stream};
use hyper;
use libra_wallet::{io_utils, wallet_library::WalletLibrary};
use logger::prelude::*;
use nextgen_crypto::ed25519::Ed25519PublicKey;
use num_traits::{
    cast::{FromPrimitive, ToPrimitive},
    identities::Zero,
};
use proto_conv::IntoProto;
use rust_decimal::Decimal;
use serde_json;
use std::{
    collections::{BTreeMap, HashMap},
    convert::TryFrom,
    fmt, fs,
    io::{stdout, Seek, SeekFrom, Write},
    path::{Display, Path},
    process::{Command, Stdio},
    str::{self, FromStr},
    sync::Arc,
    thread, time,
};
use tempfile::{NamedTempFile, TempPath};
use tokio::{self, runtime::Runtime};
use types::{
    access_path::AccessPath,
    account_address::{AccountAddress, ADDRESS_LENGTH},
    account_config::{
        account_received_event_path, account_sent_event_path, association_address,
        core_code_address, get_account_resource_or_default, AccountResource,
    },
    account_state_blob::{AccountStateBlob, AccountStateWithProof},
    contract_event::{ContractEvent, EventWithProof},
    transaction::{parse_as_transaction_argument, Program, SignedTransaction, Version},
    transaction_helpers::{create_signed_txn, TransactionSigner},
    validator_verifier::ValidatorVerifier,
};

const CLIENT_WALLET_MNEMONIC_FILE: &str = "client.mnemonic";
const GAS_UNIT_PRICE: u64 = 0;
const MAX_GAS_AMOUNT: u64 = 100_000;
const TX_EXPIRATION: i64 = 100;

/// Enum used for error formatting.
/// 用于错误格式的枚举
#[derive(Debug)]
enum InputType {
    Bool,
    UnsignedInt,
    Usize,
}

/// Account data is stored in a map and referenced by an index.
/// 帐户数据存储在map中，并由索引引用。
#[derive(Debug)]
pub struct AddressAndIndex {
    /// Address of the account.
    /// 账户地址
    pub address: AccountAddress,
    /// The account_ref_id of this account in client.
    pub index: usize,
}

/// Account is represented either as an entry into accounts vector or as an address.
/// 帐户既可以表示为帐户vector的条目，也可以表示为地址。
pub enum AccountEntry {
    /// Index into client.accounts
    Index(usize),
    /// Address of the account
    Address(AccountAddress),
}

/// Used to return the sequence and sender account index submitted for a transfer
/// 用于返回提交的转帐序列和发件人帐户索引
pub struct IndexAndSequence {
    /// Index/key of the account in TestClient::accounts vector.
    pub account_index: AccountEntry,
    /// Sequence number of the account.
    pub sequence_number: u64,
}

/// Proxy handling CLI commands/inputs.
/// 代理处理CLI命令/输入。
pub struct ClientProxy {
    /// client for admission control interface.
    /// 客户端的准入控制接口。
    pub client: GRPCClient,
    /// Created accounts.
    /// 创建账户
    pub accounts: Vec<AccountData>,
    /// Address to account_ref_id map.
    address_to_ref_id: HashMap<AccountAddress, usize>,
    /// Host that operates a faucet service 经营水龙头服务的主机
    faucet_server: String,
    /// Account used for mint operations. 用于薄荷操作的帐户。
    pub faucet_account: Option<AccountData>,
    /// Wallet library managing user accounts. 钱包库管理用户帐户。
    wallet: WalletLibrary,
    /// Whether to sync with validator on account creation.
    /// 在帐户创建时是否与验证程序同步。
    sync_on_wallet_recovery: bool,
    /// temp files (alive for duration of program)
    temp_files: Vec<TempPath>,
}

impl ClientProxy {
    /// Construct a new TestClient.
    pub fn new(
        host: &str,
        ac_port: &str,
        validator_set_file: &str,
        faucet_account_file: &str,
        sync_on_wallet_recovery: bool,
        faucet_server: Option<String>,
        mnemonic_file: Option<String>,
    ) -> Result<Self> {
        let validators_config = TrustedPeersConfig::load_config(Path::new(validator_set_file));
        let validators = validators_config.get_trusted_consensus_peers();
        ensure!(
            !validators.is_empty(),
            "Not able to load validators from trusted peers config!"
        );
        // Total 3f + 1 validators, 2f + 1 correct signatures are required.
        // If < 4 validators, all validators have to agree.
        // 总共3f+1 个验证者， 要求2f+1 正确签名。如果小于4个验证者，所有验证者必须达成一致。
        let validator_pubkeys: HashMap<AccountAddress, Ed25519PublicKey> = validators
            .into_iter()
            .map(|(key, value)| (key, value.into()))
            .collect();
        let validator_verifier = Arc::new(ValidatorVerifier::new(validator_pubkeys));
        let client = GRPCClient::new(host, ac_port, validator_verifier)?;

        let accounts = vec![];

        // If we have a faucet account file, then load it to get the keypair
        // 如果我们有一个faucet帐户文件，则将其加载以获取密钥对
        let faucet_account = if faucet_account_file.is_empty() {
            None
        } else {
            let faucet_account_keypair: KeyPair =
                ClientProxy::load_faucet_account_file(faucet_account_file);
            let faucet_account_data = Self::get_account_data_from_address(
                &client,
                association_address(),
                true,
                Some(KeyPair::new(faucet_account_keypair.private_key().clone())),
            )?;
            // Load the keypair from file
            Some(faucet_account_data)
        };

        let faucet_server = match faucet_server {
            Some(server) => server.to_string(),
            None => host.replace("ac", "faucet"),
        };

        let address_to_ref_id = accounts
            .iter()
            .enumerate()
            .map(|(ref_id, acc_data): (usize, &AccountData)| (acc_data.address, ref_id))
            .collect::<HashMap<AccountAddress, usize>>();

        Ok(ClientProxy {
            client,
            accounts,
            address_to_ref_id,
            faucet_server,
            faucet_account,
            wallet: Self::get_libra_wallet(mnemonic_file)?,
            sync_on_wallet_recovery,
            temp_files: vec![],
        })
    }

    fn get_account_ref_id(&self, sender_account_address: &AccountAddress) -> Result<usize> {
        Ok(*self
            .address_to_ref_id
            .get(&sender_account_address)
            .ok_or_else(|| {
                format_err!(
                    "Unable to find existing managing account by address: {}, to see all existing \
                     accounts, run: 'account list'",
                    sender_account_address
                )
            })?)
    }

    /// Returns the account index that should be used by user to reference this account
    /// 返回用户应用来引用该帐户的帐户索引
    pub fn create_next_account(&mut self, sync_with_validator: bool) -> Result<AddressAndIndex> {
        let (address, _) = self.wallet.new_address()?;

        let account_data =
            Self::get_account_data_from_address(&self.client, address, sync_with_validator, None)?;

        Ok(self.insert_account_data(account_data))
    }

    /// Print index and address of all accounts.
    /// 打印所有帐户的索引和地址。
    pub fn print_all_accounts(&self) {
        if self.accounts.is_empty() {
            println!("No user accounts");
        } else {
            for (ref index, ref account) in self.accounts.iter().enumerate() {
                println!(
                    "User account index: {}, address: {}, sequence number: {}, status: {:?}",
                    index,
                    hex::encode(&account.address),
                    account.sequence_number,
                    account.status,
                );
            }
        }

        if let Some(faucet_account) = &self.faucet_account {
            println!(
                "Faucet account address: {}, sequence_number: {}, status: {:?}",
                hex::encode(&faucet_account.address),
                faucet_account.sequence_number,
                faucet_account.status,
            );
        }
    }

    /// Clone all accounts held in the client.
    /// 克隆客户端中持有的所有帐户。
    pub fn copy_all_accounts(&self) -> Vec<AccountData> {
        self.accounts.clone()
    }

    /// Set the account of this client instance.
    /// 设置此客户端实例的帐户。
    pub fn set_accounts(&mut self, accounts: Vec<AccountData>) -> Vec<AddressAndIndex> {
        self.accounts.clear();
        self.address_to_ref_id.clear();
        let mut ret = vec![];
        for data in accounts {
            ret.push(self.insert_account_data(data));
        }
        ret
    }

    /// Get balance from validator for the account specified.
    /// 从验证程序获取指定帐户的余额。
    pub fn get_balance(&mut self, space_delim_strings: &[&str]) -> Result<String> {
        ensure!(
            space_delim_strings.len() == 2,
            "Invalid number of arguments for getting balance"
        );
        let address = self.get_account_address_from_parameter(space_delim_strings[1])?;
        self.get_account_resource_and_update(address).map(|res| {
            let whole_num = res.balance() / 1_000_000;
            let remainder = res.balance() % 1_000_000;
            format!("{}.{:0>6}", whole_num.to_string(), remainder.to_string())
        })
    }

    /// Get the latest sequence number from validator for the account specified.
    /// 从验证器获取指定帐户的最新序列号。
    pub fn get_sequence_number(&mut self, space_delim_strings: &[&str]) -> Result<u64> {
        ensure!(
            space_delim_strings.len() == 2 || space_delim_strings.len() == 3,
            "Invalid number of arguments for getting sequence number"
        );
        let address = self.get_account_address_from_parameter(space_delim_strings[1])?;
        let sequence_number = self
            .get_account_resource_and_update(address)?
            .sequence_number();

        let reset_sequence_number = if space_delim_strings.len() == 3 {
            parse_bool(space_delim_strings[2]).map_err(|error| {
                format_parse_data_error(
                    "reset_sequence_number",
                    InputType::Bool,
                    space_delim_strings[2],
                    error,
                )
            })?
        } else {
            false
        };
        if reset_sequence_number {
            let mut account = self.mut_account_from_parameter(space_delim_strings[1])?;
            // Set sequence_number to latest one.
            account.sequence_number = sequence_number;
        }
        Ok(sequence_number)
    }

    /// Mints coins for the receiver specified.
    /// 将sequence_number设置为最新的。
    pub fn mint_coins(&mut self, space_delim_strings: &[&str], is_blocking: bool) -> Result<()> {
        ensure!(
            space_delim_strings.len() == 3,
            "Invalid number of arguments for mint"
        );
        let receiver = self.get_account_address_from_parameter(space_delim_strings[1])?;
        let num_coins = Self::convert_to_micro_libras(space_delim_strings[2])?;

        match self.faucet_account {
            Some(_) => self.mint_coins_with_local_faucet_account(&receiver, num_coins, is_blocking),
            None => self.mint_coins_with_faucet_service(&receiver, num_coins, is_blocking),
        }
    }

    /// Waits for the next transaction for a specific address and prints it
    /// 等待下一个特定地址的交易并打印
    pub fn wait_for_transaction(&mut self, account: AccountAddress, sequence_number: u64) {
        let mut max_iterations = 5000;
        print!("[waiting ");
        loop {
            stdout().flush().unwrap();
            max_iterations -= 1;

            if let Ok(Some((_, Some(events)))) =
                self.client
                    .get_txn_by_acc_seq(account, sequence_number - 1, true)
            {
                println!("transaction is stored!");
                if events.is_empty() {
                    println!("but it didn't emit any events");
                }
                break;
            } else if max_iterations == 0 {
                panic!("wait_for_transaction timeout");
            } else {
                print!(".");
            }
            thread::sleep(time::Duration::from_millis(10));
        }
    }

    /// Transfer num_coins from sender account to receiver. If is_blocking = true,
    /// it will keep querying validator till the sequence number is bumped up in validator.
    /// 将num_coins从发送者帐户转移到接收者。 如果is_blocking = true，它将继续查询验证器，直到序列号
    /// 在验证器中增大为止。
    pub fn transfer_coins_int(
        &mut self,
        sender_account_ref_id: usize,
        receiver_address: &AccountAddress,
        num_coins: u64,
        gas_unit_price: Option<u64>,
        max_gas_amount: Option<u64>,
        is_blocking: bool,
    ) -> Result<IndexAndSequence> {
        let sender_address;
        let sender_sequence;
        {
            let sender = self.accounts.get(sender_account_ref_id).ok_or_else(|| {
                format_err!("Unable to find sender account: {}", sender_account_ref_id)
            })?;

            let program = vm_genesis::encode_transfer_program(&receiver_address, num_coins);
            let req = self.create_submit_transaction_req(
                program,
                sender,
                max_gas_amount, /* max_gas_amount */
                gas_unit_price, /* gas_unit_price */
            )?;
            let sender_mut = self
                .accounts
                .get_mut(sender_account_ref_id)
                .ok_or_else(|| {
                    format_err!("Unable to find sender account: {}", sender_account_ref_id)
                })?;
            self.client.submit_transaction(Some(sender_mut), &req)?;
            sender_address = sender_mut.address;
            sender_sequence = sender_mut.sequence_number;
        }

        if is_blocking {
            self.wait_for_transaction(sender_address, sender_sequence);
        }

        Ok(IndexAndSequence {
            account_index: AccountEntry::Index(sender_account_ref_id),
            sequence_number: sender_sequence - 1,
        })
    }

    /// Transfers coins from sender to receiver.
    pub fn transfer_coins(
        &mut self,
        space_delim_strings: &[&str],
        is_blocking: bool,
    ) -> Result<IndexAndSequence> {
        ensure!(
            space_delim_strings.len() >= 4 && space_delim_strings.len() <= 6,
            "Invalid number of arguments for transfer"
        );

        let sender_account_address =
            self.get_account_address_from_parameter(space_delim_strings[1])?;
        let receiver_address = self.get_account_address_from_parameter(space_delim_strings[2])?;

        let num_coins = Self::convert_to_micro_libras(space_delim_strings[3])?;

        let gas_unit_price = if space_delim_strings.len() > 4 {
            Some(space_delim_strings[4].parse::<u64>().map_err(|error| {
                format_parse_data_error(
                    "gas_unit_price",
                    InputType::UnsignedInt,
                    space_delim_strings[4],
                    error,
                )
            })?)
        } else {
            None
        };

        let max_gas_amount = if space_delim_strings.len() > 5 {
            Some(space_delim_strings[5].parse::<u64>().map_err(|error| {
                format_parse_data_error(
                    "max_gas_amount",
                    InputType::UnsignedInt,
                    space_delim_strings[5],
                    error,
                )
            })?)
        } else {
            None
        };

        let sender_account_ref_id = self.get_account_ref_id(&sender_account_address)?;

        self.transfer_coins_int(
            sender_account_ref_id,
            &receiver_address,
            num_coins,
            gas_unit_price,
            max_gas_amount,
            is_blocking,
        )
    }

    /// Compile move program 编译移动程序
    pub fn compile_program(&mut self, space_delim_strings: &[&str]) -> Result<String> {
        let address = self.get_account_address_from_parameter(space_delim_strings[1])?;
        let file_path = space_delim_strings[2];
        let is_module = if space_delim_strings.len() > 3 {
            parse_bool(space_delim_strings[3])?
        } else {
            false
        };
        let output_path = {
            if space_delim_strings.len() == 5 {
                space_delim_strings[4].to_string()
            } else {
                let tmp_path = NamedTempFile::new()?.into_temp_path();
                let path = tmp_path
                    .to_str()
                    .ok_or_else(|| format_err!("failed to create tmp file"))?
                    .to_string();
                self.temp_files.push(tmp_path);
                path
            }
        };
        let mut tmp_source_file = NamedTempFile::new()?;
        let mut code = fs::read_to_string(file_path)?;
        code = code.replace("{{default}}", &format!("0x{}", address));
        writeln!(tmp_source_file, "{}", code)?;

        let dependencies_file =
            self.handle_dependencies(tmp_source_file.path().display(), is_module)?;

        // custom handler of old module format
        // TODO: eventually retire code after vm separation between modules and scripts
        if is_module {
            code = format!("modules:\n{}\nscript:\nmain(){{\nreturn;\n}}", code);
            tmp_source_file.seek(SeekFrom::Start(0))?;
            writeln!(tmp_source_file, "{}", code)?;
        }

        let mut args = format!(
            "run -p compiler -- -a {} -o {} {}",
            address,
            output_path,
            tmp_source_file.path().display(),
        );
        if let Some(file) = &dependencies_file {
            args.push_str(&format!(" --deps={}", file.path().display()));
        }

        let status = Command::new("cargo")
            .args(args.split(' '))
            .spawn()?
            .wait()?;
        if !status.success() {
            return Err(format_err!("compilation failed"));
        }
        Ok(output_path)
    }

    fn handle_dependencies(
        &mut self,
        source_path: Display,
        is_module: bool,
    ) -> Result<Option<NamedTempFile>> {
        let mut args = format!("run -p compiler -- -l {}", source_path);
        if is_module {
            args.push_str(" -m");
        }
        let child = Command::new("cargo")
            .args(args.split(' '))
            .stdout(Stdio::piped())
            .spawn()?;
        let output = child.wait_with_output()?;
        let paths: Vec<AccessPath> = serde_json::from_str(str::from_utf8(&output.stdout)?)?;
        let mut dependencies = vec![];
        for path in paths {
            if path.address != core_code_address() {
                if let (Some(blob), _) = self.client.get_account_blob(path.address)? {
                    let map = BTreeMap::<Vec<u8>, Vec<u8>>::try_from(&blob)?;
                    if let Some(code) = map.get(&path.path) {
                        dependencies.push(code.clone());
                    }
                }
            }
        }
        if dependencies.is_empty() {
            return Ok(None);
        }
        let mut file = NamedTempFile::new()?;
        file.write_all(&serde_json::to_vec(&dependencies)?)?;
        Ok(Some(file))
    }

    fn submit_program(&mut self, space_delim_strings: &[&str], program: Program) -> Result<()> {
        let sender_address = self.get_account_address_from_parameter(space_delim_strings[1])?;
        let sender_ref_id = self.get_account_ref_id(&sender_address)?;
        let sender = self.accounts.get(sender_ref_id).unwrap();
        let sequence_number = sender.sequence_number;

        let req = self.create_submit_transaction_req(program, &sender, None, None)?;

        self.client
            .submit_transaction(self.accounts.get_mut(sender_ref_id), &req)?;
        self.wait_for_transaction(sender_address, sequence_number + 1);

        Ok(())
    }

    /// Publish move module
    pub fn publish_module(&mut self, space_delim_strings: &[&str]) -> Result<()> {
        let program = serde_json::from_slice(&fs::read(space_delim_strings[2])?)?;
        self.submit_program(space_delim_strings, program)
    }

    /// Execute custom script 执行自定义脚本
    pub fn execute_script(&mut self, space_delim_strings: &[&str]) -> Result<()> {
        let program: Program = serde_json::from_slice(&fs::read(space_delim_strings[2])?)?;
        let arguments: Vec<_> = space_delim_strings[3..]
            .iter()
            .filter_map(|arg| parse_as_transaction_argument(arg).ok())
            .collect();
        let (script, _, modules) = program.into_inner();
        self.submit_program(
            space_delim_strings,
            Program::new(script, modules, arguments),
        )
    }

    /// Get the latest account state from validator.
    /// 从验证器获取最新的帐户状态。
    pub fn get_latest_account_state(
        &mut self,
        space_delim_strings: &[&str],
    ) -> Result<(Option<AccountStateBlob>, Version)> {
        ensure!(
            space_delim_strings.len() == 2,
            "Invalid number of arguments to get latest account state"
        );
        let account = self.get_account_address_from_parameter(space_delim_strings[1])?;
        self.get_account_state_and_update(account)
    }

    /// Get committed txn by account and sequence number.
    /// 通过帐户和序列号获取提交的txn。
    pub fn get_committed_txn_by_acc_seq(
        &mut self,
        space_delim_strings: &[&str],
    ) -> Result<Option<(SignedTransaction, Option<Vec<ContractEvent>>)>> {
        ensure!(
            space_delim_strings.len() == 4,
            "Invalid number of arguments to get transaction by account and sequence number"
        );
        let account = self.get_account_address_from_parameter(space_delim_strings[1])?;
        let sequence_number = space_delim_strings[2].parse::<u64>().map_err(|error| {
            format_parse_data_error(
                "account_sequence_number",
                InputType::UnsignedInt,
                space_delim_strings[2],
                error,
            )
        })?;

        let fetch_events = parse_bool(space_delim_strings[3]).map_err(|error| {
            format_parse_data_error(
                "fetch_events",
                InputType::Bool,
                space_delim_strings[3],
                error,
            )
        })?;

        self.client
            .get_txn_by_acc_seq(account, sequence_number, fetch_events)
    }

    /// Get committed txn by account and sequence number
    /// 通过帐户和序列号获取提交的TXN
    pub fn get_committed_txn_by_range(
        &mut self,
        space_delim_strings: &[&str],
    ) -> Result<Vec<(SignedTransaction, Option<Vec<ContractEvent>>)>> {
        ensure!(
            space_delim_strings.len() == 4,
            "Invalid number of arguments to get transaction by range"
        );
        let start_version = space_delim_strings[1].parse::<u64>().map_err(|error| {
            format_parse_data_error(
                "start_version",
                InputType::UnsignedInt,
                space_delim_strings[1],
                error,
            )
        })?;
        let limit = space_delim_strings[2].parse::<u64>().map_err(|error| {
            format_parse_data_error(
                "limit",
                InputType::UnsignedInt,
                space_delim_strings[2],
                error,
            )
        })?;
        let fetch_events = parse_bool(space_delim_strings[3]).map_err(|error| {
            format_parse_data_error(
                "fetch_events",
                InputType::Bool,
                space_delim_strings[3],
                error,
            )
        })?;

        self.client
            .get_txn_by_range(start_version, limit, fetch_events)
    }

    /// Get account address from parameter. If the parameter is string of address, try to convert
    /// it to address, otherwise, try to convert to u64 and looking at TestClient::accounts.
    /// 从参数获取帐户地址。 如果参数是地址字符串，请尝试将其转换为地址，否则，请尝试转换为u64并
    /// 查看TestClient :: accounts。
    pub fn get_account_address_from_parameter(&self, para: &str) -> Result<AccountAddress> {
        match is_address(para) {
            true => ClientProxy::address_from_strings(para),
            false => {
                let account_ref_id = para.parse::<usize>().map_err(|error| {
                    format_parse_data_error(
                        "account_reference_id/account_address",
                        InputType::Usize,
                        para,
                        error,
                    )
                })?;
                let account_data = self.accounts.get(account_ref_id).ok_or_else(|| {
                    format_err!(
                        "Unable to find account by account reference id: {}, to see all existing \
                         accounts, run: 'account list'",
                        account_ref_id
                    )
                })?;
                Ok(account_data.address)
            }
        }
    }

    /// Get events by account and event type with start sequence number and limit.
    /// 按帐户和事件类型获取事件，并带有开始序列号和限制。
    pub fn get_events_by_account_and_type(
        &mut self,
        space_delim_strings: &[&str],
    ) -> Result<(Vec<EventWithProof>, Option<AccountStateWithProof>)> {
        ensure!(
            space_delim_strings.len() == 6,
            "Invalid number of arguments to get events by access path"
        );
        let account = self.get_account_address_from_parameter(space_delim_strings[1])?;
        let path = match space_delim_strings[2] {
            "sent" => account_sent_event_path(),
            "received" => account_received_event_path(),
            _ => bail!(
                "Unknown event type: {:?}, only sent and received are supported",
                space_delim_strings[2]
            ),
        };
        let access_path = AccessPath::new(account, path);
        let start_seq_number = space_delim_strings[3].parse::<u64>().map_err(|error| {
            format_parse_data_error(
                "start_seq_number",
                InputType::UnsignedInt,
                space_delim_strings[3],
                error,
            )
        })?;
        let ascending = parse_bool(space_delim_strings[4]).map_err(|error| {
            format_parse_data_error("ascending", InputType::Bool, space_delim_strings[4], error)
        })?;
        let limit = space_delim_strings[5].parse::<u64>().map_err(|error| {
            format_parse_data_error(
                "start_seq_number",
                InputType::UnsignedInt,
                space_delim_strings[3],
                error,
            )
        })?;
        self.client
            .get_events_by_access_path(access_path, start_seq_number, ascending, limit)
    }

    /// Write mnemonic recover to the file specified.
    /// 将助记符恢复写入指定的文件。
    pub fn write_recovery(&self, space_delim_strings: &[&str]) -> Result<()> {
        ensure!(
            space_delim_strings.len() == 2,
            "Invalid number of arguments for writing recovery"
        );

        self.wallet
            .write_recovery(&Path::new(space_delim_strings[1]))?;
        Ok(())
    }

    /// Recover wallet accounts from file and return vec<(account_address, index)>.
    /// 从文件中恢复钱包帐户并返回vec <（account_address，index）>。
    pub fn recover_wallet_accounts(
        &mut self,
        space_delim_strings: &[&str],
    ) -> Result<Vec<AddressAndIndex>> {
        ensure!(
            space_delim_strings.len() == 2,
            "Invalid number of arguments for recovering wallets"
        );

        let wallet = WalletLibrary::recover(&Path::new(space_delim_strings[1]))?;
        let wallet_addresses = wallet.get_addresses()?;
        let mut account_data = Vec::new();
        for address in wallet_addresses {
            account_data.push(Self::get_account_data_from_address(
                &self.client,
                address,
                self.sync_on_wallet_recovery,
                None,
            )?);
        }
        self.set_wallet(wallet);
        // Clear current cached AccountData as we always swap the entire wallet completely.
        Ok(self.set_accounts(account_data))
    }

    /// Insert the account data to Client::accounts and return its address and index.s
    pub fn insert_account_data(&mut self, account_data: AccountData) -> AddressAndIndex {
        let address = account_data.address;

        self.accounts.push(account_data);
        self.address_to_ref_id
            .insert(address, self.accounts.len() - 1);

        AddressAndIndex {
            address,
            index: self.accounts.len() - 1,
        }
    }

    /// Test gRPC client connection with validator.
    pub fn test_validator_connection(&self) -> Result<()> {
        self.client.get_with_proof_sync(vec![])?;
        Ok(())
    }

    /// Get account state from validator and update status of account if it is cached locally.
    fn get_account_state_and_update(
        &mut self,
        address: AccountAddress,
    ) -> Result<(Option<AccountStateBlob>, Version)> {
        let account_state = self.client.get_account_blob(address)?;
        if self.address_to_ref_id.contains_key(&address) {
            let account_ref_id = self
                .address_to_ref_id
                .get(&address)
                .expect("Should have the key");
            let mut account_data: &mut AccountData =
                self.accounts.get_mut(*account_ref_id).unwrap_or_else(|| panic!("Local cache not consistent, reference id {} not available in local accounts", account_ref_id));
            if account_state.0.is_some() {
                account_data.status = AccountStatus::Persisted;
            }
        };
        Ok(account_state)
    }

    /// Get account resource from validator and update status of account if it is cached locally.
    fn get_account_resource_and_update(
        &mut self,
        address: AccountAddress,
    ) -> Result<AccountResource> {
        let account_state = self.get_account_state_and_update(address)?;
        get_account_resource_or_default(&account_state.0)
    }

    /// Get account using specific address.
    /// Sync with validator for account sequence number in case it is already created on chain.
    /// This assumes we have a very low probability of mnemonic word conflict.
    /// 使用特定地址获取帐户，如果已在链上创建，则与验证程序同步以获取帐户序列号。 假设我们记忆单词冲突的可能性非常低。
    fn get_account_data_from_address(
        client: &GRPCClient,
        address: AccountAddress,
        sync_with_validator: bool,
        key_pair: Option<KeyPair>,
    ) -> Result<AccountData> {
        let (sequence_number, status) = match sync_with_validator {
            true => match client.get_account_blob(address) {
                Ok(resp) => match resp.0 {
                    Some(account_state_blob) => (
                        get_account_resource_or_default(&Some(account_state_blob))?
                            .sequence_number(),
                        AccountStatus::Persisted,
                    ),
                    None => (0, AccountStatus::Local),
                },
                Err(e) => {
                    error!("Failed to get account state from validator, error: {:?}", e);
                    (0, AccountStatus::Unknown)
                }
            },
            false => (0, AccountStatus::Local),
        };
        Ok(AccountData {
            address,
            key_pair,
            sequence_number,
            status,
        })
    }

    fn get_libra_wallet(mnemonic_file: Option<String>) -> Result<WalletLibrary> {
        let wallet_recovery_file_path = if let Some(input_mnemonic_word) = mnemonic_file {
            Path::new(&input_mnemonic_word).to_path_buf()
        } else {
            let mut file_path = std::env::current_dir()?;
            file_path.push(CLIENT_WALLET_MNEMONIC_FILE);
            file_path
        };

        let wallet = if let Ok(recovered_wallet) = io_utils::recover(&wallet_recovery_file_path) {
            recovered_wallet
        } else {
            let new_wallet = WalletLibrary::new();
            new_wallet.write_recovery(&wallet_recovery_file_path)?;
            new_wallet
        };
        Ok(wallet)
    }

    /// Set wallet instance used by this client. 设置该客户端使用的钱包实例。
    fn set_wallet(&mut self, wallet: WalletLibrary) {
        self.wallet = wallet;
    }

    fn load_faucet_account_file(faucet_account_file: &str) -> KeyPair {
        match fs::read(faucet_account_file) {
            Ok(data) => {
                bincode::deserialize(&data[..]).expect("Unable to deserialize faucet account file")
            }
            Err(e) => {
                panic!(
                    "Unable to read faucet account file: {}, {}",
                    faucet_account_file, e
                );
            }
        }
    }

    fn address_from_strings(data: &str) -> Result<AccountAddress> {
        let account_vec: Vec<u8> = hex::decode(data.parse::<String>()?)?;
        ensure!(
            account_vec.len() == ADDRESS_LENGTH,
            "The address {:?} is of invalid length. Addresses must be 32-bytes long"
        );
        let account = match AccountAddress::try_from(&account_vec[..]) {
            Ok(address) => address,
            Err(error) => bail!(
                "The address {:?} is invalid, error: {:?}",
                &account_vec,
                error,
            ),
        };
        Ok(account)
    }

    fn mint_coins_with_local_faucet_account(
        &mut self,
        receiver: &AccountAddress,
        num_coins: u64,
        is_blocking: bool,
    ) -> Result<()> {
        ensure!(self.faucet_account.is_some(), "No faucet account loaded");
        let sender = self.faucet_account.as_ref().unwrap();
        let sender_address = sender.address;
        let program = vm_genesis::encode_mint_program(&receiver, num_coins);
        let req = self.create_submit_transaction_req(
            program, sender, None, /* max_gas_amount */
            None, /* gas_unit_price */
        )?;
        let mut sender_mut = self.faucet_account.as_mut().unwrap();
        let resp = self.client.submit_transaction(Some(&mut sender_mut), &req);
        if is_blocking {
            self.wait_for_transaction(
                sender_address,
                self.faucet_account.as_ref().unwrap().sequence_number,
            );
        }
        resp
    }

    fn mint_coins_with_faucet_service(
        &mut self,
        receiver: &AccountAddress,
        num_coins: u64,
        is_blocking: bool,
    ) -> Result<()> {
        let mut runtime = Runtime::new().unwrap();
        let client = hyper::Client::new();

        let url = format!(
            "http://{}?amount={}&address={:?}",
            self.faucet_server, num_coins, receiver
        )
        .parse::<hyper::Uri>()?;

        let request = hyper::Request::post(url).body(hyper::Body::empty())?;
        let response = runtime.block_on(client.request(request))?;
        let status_code = response.status();
        let body = response.into_body().concat2().wait()?;
        let raw_data = std::str::from_utf8(&body)?;

        if status_code != 200 {
            return Err(format_err!(
                "Failed to query remote faucet server[status={}]: {:?}",
                status_code,
                raw_data,
            ));
        }
        let sequence_number = raw_data.parse::<u64>()?;
        if is_blocking {
            self.wait_for_transaction(AccountAddress::new([0; 32]), sequence_number);
        }
        Ok(())
    }

    fn convert_to_micro_libras(input: &str) -> Result<u64> {
        ensure!(!input.is_empty(), "Empty input not allowed for libra unit");
        // This is not supposed to panic as it is used as constant here. 这不应该引起恐慌，因为在此处将其用作常量。
        let max_value = Decimal::from_u64(std::u64::MAX).unwrap() / Decimal::new(1_000_000, 0);
        let scale = input.find('.').unwrap_or(input.len() - 1);
        ensure!(
            scale <= 14,
            "Input value is too big: {:?}, max: {:?}",
            input,
            max_value
        );
        let original = Decimal::from_str(input)?;
        ensure!(
            original <= max_value,
            "Input value is too big: {:?}, max: {:?}",
            input,
            max_value
        );
        let value = original * Decimal::new(1_000_000, 0);
        ensure!(value.fract().is_zero(), "invalid value");
        value.to_u64().ok_or_else(|| format_err!("invalid value"))
    }

    /// Craft a transaction request. 制定交易请求。
    fn create_submit_transaction_req(
        &self,
        program: Program,
        sender_account: &AccountData,
        max_gas_amount: Option<u64>,
        gas_unit_price: Option<u64>,
    ) -> Result<SubmitTransactionRequest> {
        let signer: Box<&dyn TransactionSigner> = match &sender_account.key_pair {
            Some(key_pair) => Box::new(key_pair),
            None => Box::new(&self.wallet),
        };
        let signed_txn = create_signed_txn(
            *signer,
            program,
            sender_account.address,
            sender_account.sequence_number,
            max_gas_amount.unwrap_or(MAX_GAS_AMOUNT),
            gas_unit_price.unwrap_or(GAS_UNIT_PRICE),
            TX_EXPIRATION,
        )
        .unwrap();
        let mut req = SubmitTransactionRequest::new();
        req.set_signed_txn(signed_txn.into_proto());
        Ok(req)
    }

    fn mut_account_from_parameter(&mut self, para: &str) -> Result<&mut AccountData> {
        let account_ref_id = match is_address(para) {
            true => {
                let account_address = ClientProxy::address_from_strings(para)?;
                *self
                    .address_to_ref_id
                    .get(&account_address)
                    .ok_or_else(|| {
                        format_err!(
                            "Unable to find local account by address: {:?}",
                            account_address
                        )
                    })?
            }
            false => para.parse::<usize>()?,
        };
        let account_data = self
            .accounts
            .get_mut(account_ref_id)
            .ok_or_else(|| format_err!("Unable to find account by ref id: {}", account_ref_id))?;
        Ok(account_data)
    }
}

fn format_parse_data_error<T: std::fmt::Debug>(
    field: &str,
    input_type: InputType,
    value: &str,
    error: T,
) -> Error {
    format_err!(
        "Unable to parse input for {} - \
         please enter an {:?}.  Input was: {}, error: {:?}",
        field,
        input_type,
        value,
        error
    )
}

fn parse_bool(para: &str) -> Result<bool> {
    Ok(para.to_lowercase().parse::<bool>()?)
}

impl fmt::Display for AccountEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AccountEntry::Index(i) => write!(f, "{}", i),
            AccountEntry::Address(addr) => write!(f, "{}", addr),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::client_proxy::{parse_bool, AddressAndIndex, ClientProxy};
    use config::trusted_peers::TrustedPeersConfigHelpers;
    use libra_wallet::io_utils;
    use proptest::prelude::*;
    use tempfile::NamedTempFile;

    fn generate_accounts_from_wallet(count: usize) -> (ClientProxy, Vec<AddressAndIndex>) {
        let mut accounts = Vec::new();
        accounts.reserve(count);
        let file = NamedTempFile::new().unwrap();
        let mnemonic_path = file.into_temp_path().to_str().unwrap().to_string();
        let trust_peer_file = NamedTempFile::new().unwrap();
        let (_, trust_peer_config) = TrustedPeersConfigHelpers::get_test_config(1, None);
        let trust_peer_path = trust_peer_file.into_temp_path();
        trust_peer_config.save_config(&trust_peer_path);

        let val_set_file = trust_peer_path.to_str().unwrap().to_string();

        // We don't need to specify host/port since the client won't be used to connect, only to
        // generate random accounts
        let mut client_proxy = ClientProxy::new(
            "", /* host */
            "", /* port */
            &val_set_file,
            &"",
            false,
            None,
            Some(mnemonic_path),
        )
        .unwrap();
        for _ in 0..count {
            accounts.push(client_proxy.create_next_account(false).unwrap());
        }

        (client_proxy, accounts)
    }

    #[test]
    fn test_parse_bool() {
        assert!(parse_bool("true").unwrap());
        assert!(parse_bool("True").unwrap());
        assert!(parse_bool("TRue").unwrap());
        assert!(parse_bool("TRUE").unwrap());
        assert!(!parse_bool("false").unwrap());
        assert!(!parse_bool("False").unwrap());
        assert!(!parse_bool("FaLSe").unwrap());
        assert!(!parse_bool("FALSE").unwrap());
        assert!(parse_bool("1").is_err());
        assert!(parse_bool("0").is_err());
        assert!(parse_bool("2").is_err());
        assert!(parse_bool("1adf").is_err());
        assert!(parse_bool("ad13").is_err());
        assert!(parse_bool("ad1f").is_err());
    }

    #[test]
    fn test_micro_libra_conversion() {
        assert!(ClientProxy::convert_to_micro_libras("").is_err());
        assert!(ClientProxy::convert_to_micro_libras("-11").is_err());
        assert!(ClientProxy::convert_to_micro_libras("abc").is_err());
        assert!(ClientProxy::convert_to_micro_libras("11111112312321312321321321").is_err());
        assert!(ClientProxy::convert_to_micro_libras("0").is_ok());
        assert!(ClientProxy::convert_to_micro_libras("1").is_ok());
        assert!(ClientProxy::convert_to_micro_libras("0.1").is_ok());
        assert!(ClientProxy::convert_to_micro_libras("1.1").is_ok());
        // Max of micro libra is u64::MAX (18446744073709551615).
        assert!(ClientProxy::convert_to_micro_libras("18446744073709.551615").is_ok());
        assert!(ClientProxy::convert_to_micro_libras("184467440737095.51615").is_err());
        assert!(ClientProxy::convert_to_micro_libras("18446744073709.551616").is_err());
    }

    #[test]
    fn test_generate() {
        let num = 1;
        let (_, accounts) = generate_accounts_from_wallet(num);
        assert_eq!(accounts.len(), num);
    }

    #[test]
    fn test_write_recover() {
        let num = 100;
        let (client, accounts) = generate_accounts_from_wallet(num);
        assert_eq!(accounts.len(), num);

        let file = NamedTempFile::new().unwrap();
        let path = file.into_temp_path();
        io_utils::write_recovery(&client.wallet, &path).expect("failed to write to file");

        let wallet = io_utils::recover(&path).expect("failed to load from file");

        assert_eq!(client.wallet.mnemonic(), wallet.mnemonic());
    }

    proptest! {
        // Proptest is used to verify that the conversion will not panic with random input.
        #[test]
        fn test_micro_libra_conversion_random_string(req in any::<String>()) {
            let _res = ClientProxy::convert_to_micro_libras(&req);
        }
        #[test]
        fn test_micro_libra_conversion_random_f64(req in any::<f64>()) {
            let req_str = req.to_string();
            let _res = ClientProxy::convert_to_micro_libras(&req_str);
        }
        #[test]
        fn test_micro_libra_conversion_random_u64(req in any::<u64>()) {
            let req_str = req.to_string();
            let _res = ClientProxy::convert_to_micro_libras(&req_str);
        }
    }
}
