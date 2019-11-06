// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! A module to generate, store and load known users accounts.
//! The concept of known users can be helpful for testing to provide reproducible results.
//! 生成，存储和加载已知用户帐户的模块。
//! 已知用户的概念可能有助于测试以提供可重现的结果。

use crate::*;
use failure::prelude::*;
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    path::Path,
};

/// Delimiter used to ser/deserialize account data.
/// 用于对帐户数据进行序列化/反序列化的定界符。
pub const DELIMITER: &str = ";";

/// Recover wallet from the path specified.
/// 从指定的路径中恢复钱包。
pub fn recover<P: AsRef<Path>>(path: &P) -> Result<WalletLibrary> {
    let input = File::open(path)?;
    let mut buffered = BufReader::new(input);

    let mut line = String::new();
    let _ = buffered.read_line(&mut line)?;
    let parts: Vec<&str> = line.split(DELIMITER).collect();
    ensure!(parts.len() == 2, format!("Invalid entry '{}'", line));

    let mnemonic = Mnemonic::from(&parts[0].to_string()[..])?;
    let mut wallet = WalletLibrary::new_from_mnemonic(mnemonic);
    wallet.generate_addresses(parts[1].trim().to_string().parse::<u64>()?)?;

    Ok(wallet)
}

/// Write wallet seed to file.
/// 将钱包种子写入文件。
pub fn write_recovery<P: AsRef<Path>>(wallet: &WalletLibrary, path: &P) -> Result<()> {
    let mut output = File::create(path)?;
    writeln!(
        output,
        "{}{}{}",
        wallet.mnemonic().to_string(),
        DELIMITER,
        wallet.key_leaf()
    )?;

    Ok(())
}
