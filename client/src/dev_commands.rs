// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{client_proxy::ClientProxy, commands::*};

/// Major command for account related operations.
/// 帐户相关操作的主要命令。
pub struct DevCommand {}

impl Command for DevCommand {
    fn get_aliases(&self) -> Vec<&'static str> {
        vec!["dev"]
    }
    fn get_description(&self) -> &'static str {
        "Local move development"
    }
    fn execute(&self, client: &mut ClientProxy, params: &[&str]) {
        let commands: Vec<Box<dyn Command>> = vec![
            Box::new(DevCommandCompile {}),
            Box::new(DevCommandPublish {}),
            Box::new(DevCommandExecute {}),
        ];
        subcommand_execute(&params[0], commands, client, &params[1..]);
    }
}

/// Sub command to compile move program
/// 子命令编译移动程序
pub struct DevCommandCompile {}

impl Command for DevCommandCompile {
    fn get_aliases(&self) -> Vec<&'static str> {
        vec!["compile", "c"]
    }
    fn get_params_help(&self) -> &'static str {
        "<sender_account_address>|<sender_account_ref_id> <file_path> [is_module (default=false)] [output_file_path (compile into tmp file by default)]"
    }
    fn get_description(&self) -> &'static str {
        "Compile move program"
    }
    fn execute(&self, client: &mut ClientProxy, params: &[&str]) {
        if params.len() < 3 || params.len() > 5 {
            println!("Invalid number of arguments for compilation");
            return;
        }
        println!(">> Compiling program");
        match client.compile_program(params) {
            Ok(path) => println!("Successfully compiled a program at {}", path),
            Err(e) => println!("{}", e),
        }
    }
}

/// Sub command to publish move resource
/// 子命令发布移动资源
pub struct DevCommandPublish {}

impl Command for DevCommandPublish {
    fn get_aliases(&self) -> Vec<&'static str> {
        vec!["publish", "p"]
    }

    fn get_params_help(&self) -> &'static str {
        "<sender_account_address>|<sender_account_ref_id> <compiled_module_path>"
    }

    fn get_description(&self) -> &'static str {
        "Publish move module on-chain"
    }

    fn execute(&self, client: &mut ClientProxy, params: &[&str]) {
        if params.len() != 3 {
            println!("Invalid number of arguments to publish module");
            return;
        }
        match client.publish_module(params) {
            Ok(_) => println!("Successfully published module"),
            Err(e) => println!("{}", e),
        }
    }
}

/// Sub command to execute custom move script
/// 子命令执行自定义移动脚本
pub struct DevCommandExecute {}

impl Command for DevCommandExecute {
    fn get_aliases(&self) -> Vec<&'static str> {
        vec!["execute", "e"]
    }

    fn get_params_help(&self) -> &'static str {
        "<sender_account_address>|<sender_account_ref_id> <compiled_module_path> [parameters]"
    }

    fn get_description(&self) -> &'static str {
        "Execute custom move script"
    }

    fn execute(&self, client: &mut ClientProxy, params: &[&str]) {
        if params.len() < 3 {
            println!("Invalid number of arguments to execute script");
            return;
        }
        match client.execute_script(params) {
            Ok(_) => println!("Successfully finished execution"),
            Err(e) => println!("{}", e),
        }
    }
}
