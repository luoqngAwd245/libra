// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    account_commands::AccountCommand, client_proxy::ClientProxy, dev_commands::DevCommand,
    query_commands::QueryCommand, transfer_commands::TransferCommand,
};

use failure::prelude::*;
use metrics::counters::*;
use std::{collections::HashMap, sync::Arc};
use types::account_address::ADDRESS_LENGTH;

/// Print the error and bump up error counter.
/// 打印错误并增加错误计数器。
pub fn report_error(msg: &str, e: Error) {
    println!("[ERROR] {}: {}", msg, pretty_format_error(e));
    COUNTER_CLIENT_ERRORS.inc();
}

fn pretty_format_error(e: Error) -> String {
    if let Some(grpc_error) = e.downcast_ref::<grpcio::Error>() {
        if let grpcio::Error::RpcFailure(grpc_rpc_failure) = grpc_error {
            match grpc_rpc_failure.status {
                grpcio::RpcStatusCode::Unavailable | grpcio::RpcStatusCode::DeadlineExceeded => {
                    return "Server unavailable, please retry and/or check \
                            if host passed to the client is running"
                        .to_string();
                }
                _ => {}
            }
        }
    }

    return format!("{}", e);
}

/// Check whether a command is blocking.
/// 检查命令是否阻塞
pub fn blocking_cmd(cmd: &str) -> bool {
    cmd.ends_with('b')
}

/// Check whether a command is debugging command.
/// 检查命令是否为调试命令。
pub fn debug_format_cmd(cmd: &str) -> bool {
    cmd.ends_with('?')
}

/// Check whether the input string is a valid libra address.
/// 检查输入的字符串是否为有效的天秤地址。
pub fn is_address(data: &str) -> bool {
    match hex::decode(data) {
        Ok(vec) => vec.len() == ADDRESS_LENGTH,
        Err(_) => false,
    }
}

/// Returns all the commands available, as well as the reverse index from the aliases to the
/// commands.
/// 返回所有可用命令，同时从别名到命令的反向索引
pub fn get_commands() -> (
    Vec<Arc<dyn Command>>,
    HashMap<&'static str, Arc<dyn Command>>,
) {
    let commands: Vec<Arc<dyn Command>> = vec![
        Arc::new(AccountCommand {}),
        Arc::new(QueryCommand {}),
        Arc::new(TransferCommand {}),
        Arc::new(DevCommand {}),
    ];
    let mut alias_to_cmd = HashMap::new();
    for command in &commands {
        for alias in command.get_aliases() {
            alias_to_cmd.insert(alias, Arc::clone(command));
        }
    }
    (commands, alias_to_cmd)
}

/// Parse a cmd string, the first element in the returned vector is the command to run
/// 解析一个cmd字符串，返回vector中的第一个元素是要运行的命令
pub fn parse_cmd(cmd_str: &str) -> Vec<&str> {
    cmd_str.split_ascii_whitespace().collect()
}

/// Print the help message for all sub commands.
/// 打印所有子命令的帮助消息。
pub fn print_subcommand_help(parent_command: &str, commands: &[Box<dyn Command>]) {
    println!(
        "usage: {} <arg>\n\nUse the following args for this command:\n",
        parent_command
    );
    for cmd in commands {
        println!(
            "{} {}\n\t{}",
            cmd.get_aliases().join(" | "),
            cmd.get_params_help(),
            cmd.get_description()
        );
    }
    println!("\n");
}

/// Execute sub command.
// TODO: Convert subcommands arrays to lazy statics
pub fn subcommand_execute(
    parent_command_name: &str,
    commands: Vec<Box<dyn Command>>,
    client: &mut ClientProxy,
    params: &[&str],
) {
    let mut commands_map = HashMap::new();
    for (i, cmd) in commands.iter().enumerate() {
        for alias in cmd.get_aliases() {
            if commands_map.insert(alias, i) != None {
                panic!("Duplicate alias {}", alias);
            }
        }
    }

    if params.is_empty() {
        print_subcommand_help(parent_command_name, &commands);
        return;
    }

    match commands_map.get(&params[0]) {
        Some(&idx) => commands[idx].execute(client, &params),
        _ => print_subcommand_help(parent_command_name, &commands),
    }
}

/// Trait to perform client operations.
pub trait Command {
    /// all commands and aliases this command support.
    fn get_aliases(&self) -> Vec<&'static str>;
    /// string that describes params.
    fn get_params_help(&self) -> &'static str {
        ""
    }
    /// string that describes what the command does.
    fn get_description(&self) -> &'static str;
    /// code to execute.
    fn execute(&self, client: &mut ClientProxy, params: &[&str]);
}
