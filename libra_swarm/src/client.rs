// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::utils;
use client_lib::{client_proxy::ClientProxy, commands};
use std::{
    collections::HashMap,
    io::{self, Write},
    path::Path,
    process::{Child, Command, Output, Stdio},
    sync::Arc,
};

/// 交互客户端
pub struct InteractiveClient {
    client: Option<Child>,
}

impl Drop for InteractiveClient {
    fn drop(&mut self) {
        if self.client.is_none() {
            return;
        }
        // Kill client process if still running. 如果还在运行杀死进程
        let mut client = self.client.take().unwrap();
        match client.try_wait().unwrap() {
            Some(status) => {
                if !status.success() {
                    panic!(
                        "Client terminated with status: {}",
                        status.code().unwrap_or(-1)
                    );
                }
            }
            None => {
                client.kill().unwrap();
            }
        }
    }
}

impl InteractiveClient {
    //从继承io创建
    pub fn new_with_inherit_io(
        port: u16,
        faucet_key_file_path: &Path,
        mnemonic_file_path: &Path,
        validator_set_file: String,
    ) -> Self {
        // We need to call canonicalize on the path because we are running client from
        // workspace root and the function calling new_with_inherit_io isn't necessarily
        // running from that location, so if a relative path is passed, it wouldn't work
        // unless we convert it to an absolute path
        // 我们需要在路径上调用canonicalize，因为我们从工作空间根目录运行客户端，并且调用new_with_inherit_io
        // 的函数不一定从该位置运行，因此如果传递相对路径，除非我们将其转换为绝对路径，否则它将无法工作
        Self {
            client: Some(
                Command::new(utils::get_bin("client"))
                    .current_dir(utils::workspace_root())
                    .arg("-p")
                    .arg(port.to_string())
                    .arg("-m")
                    .arg(
                        faucet_key_file_path
                            .canonicalize()
                            .expect("Unable to get canonical path of faucet key file")
                            .to_str()
                            .unwrap(),
                    )
                    .arg("-n")
                    .arg(
                        mnemonic_file_path
                            .canonicalize()
                            .expect("Unable to get canonical path of mnemonic file")
                            .to_str()
                            .unwrap(),
                    )
                    .arg("-a")
                    .arg("localhost")
                    .arg("-s")
                    .arg(validator_set_file)
                    .stdin(Stdio::inherit())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .spawn()
                    .expect("Failed to spawn client process"),
            ),
        }
    }
    //从管道io创建
    pub fn new_with_piped_io(
        port: u16,
        faucet_key_file_path: &Path,
        mnemonic_file_path: &Path,
        validator_set_file: String,
    ) -> Self {
        Self {
            /// Note: For easier debugging it's convenient to see the output
            /// from the client CLI. Comment the stdout/stderr lines below
            /// and enjoy pretty Matrix-style output.
            /// 注意：为了便于调试，可以方便地查看客户端CLI的输出。 注释下面的stdout / stderr行，
            /// 享受漂亮的Matrix风格输出。
            client: Some(
                Command::new(utils::get_bin("client"))
                    .current_dir(utils::workspace_root())
                    .arg("-p")
                    .arg(port.to_string())
                    .arg("-m")
                    .arg(
                        faucet_key_file_path
                            .canonicalize()
                            .expect("Unable to get canonical path of faucet key file")
                            .to_str()
                            .unwrap(),
                    )
                    .arg("-n")
                    .arg(
                        mnemonic_file_path
                            .canonicalize()
                            .expect("Unable to get canonical path of mnemonic file")
                            .to_str()
                            .unwrap(),
                    )
                    .arg("-a")
                    .arg("localhost")
                    .arg("-s")
                    .arg(validator_set_file)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("Failed to spawn client process"),
            ),
        }
    }

    pub fn output(mut self) -> io::Result<Output> {
        self.client.take().unwrap().wait_with_output()
    }

    pub fn send_instructions(&mut self, instructions: &[&str]) -> io::Result<()> {
        let input = self.client.as_mut().unwrap().stdin.as_mut().unwrap();
        for i in instructions {
            input.write_all((i.to_string() + "\n").as_bytes())?;
            input.flush()?;
        }
        Ok(())
    }
}

pub struct InProcessTestClient {
    client: ClientProxy,
    alias_to_cmd: HashMap<&'static str, Arc<dyn commands::Command>>,
}

impl InProcessTestClient {
    pub fn new(
        port: u16,
        faucet_key_file_path: &Path,
        mnemonic_file_path: &str,
        validator_set_file: String,
    ) -> Self {
        let (_, alias_to_cmd) = commands::get_commands();
        Self {
            client: ClientProxy::new(
                "localhost",
                port.to_string().as_str(),
                &validator_set_file,
                faucet_key_file_path
                    .canonicalize()
                    .expect("Unable to get canonical path of faucet key file")
                    .to_str()
                    .unwrap(),
                false,
                /* faucet server */ None,
                Some(mnemonic_file_path.to_string()),
            )
            .unwrap(),
            alias_to_cmd,
        }
    }

    pub fn execute_instructions(&mut self, instructions: &[&str]) {
        for instr in instructions {
            let to_parse = &instr.to_string();
            let params = commands::parse_cmd(to_parse);
            // filter out empty lines
            if params.is_empty() || params[0].is_empty() {
                continue;
            }
            let cmd = self.alias_to_cmd.get(params[0]).expect("Cmd not found");
            cmd.execute(&mut self.client, &params);
        }
    }

    pub fn client(&mut self) -> &mut ClientProxy {
        &mut self.client
    }
}
