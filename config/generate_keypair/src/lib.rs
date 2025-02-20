// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use bincode::serialize;
use crypto::signing::KeyPair;
use failure::prelude::*;
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};
use tempdir::TempDir;

pub fn create_faucet_key_file(output_file: &str) -> KeyPair {
    let output_file_path = Path::new(&output_file);

    if output_file_path.exists() && !output_file_path.is_file() {
        panic!("Specified output file path is a directory");
    }

    let (private_key, _) = ::crypto::signing::generate_keypair();
    let keypair = KeyPair::new(private_key);

    // Write to disk
    let encoded: Vec<u8> = serialize(&keypair).expect("Unable to serialize keys");
    let mut file =
        File::create(output_file_path).expect("Unable to create/truncate file at specified path");
    file.write_all(&encoded)
        .expect("Unable to write keys to file at specified path");
    keypair
}

/// Tries to load a keypair from the path given as argument
/// 尝试从以参数形式给定的路径中加载keypair
pub fn load_key_from_file<P: AsRef<Path>>(path: P) -> Result<KeyPair> {
    bincode::deserialize(&fs::read(path)?[..]).map_err(|b| b.into())
}

/// Returns the generated or loaded keypair, the path to the file where this keypair is saved,
/// and a reference to the temp directory that was possibly created (a handle so that
/// it doesn't go out of scope)
/// 返回生成或加载的keypair, keypair保存的路径，可能的临时目录的引用（一个句柄以至于不超出作用域）
pub fn load_faucet_key_or_create_default(
    file_path: Option<String>,
) -> (KeyPair, String, Option<TempDir>) {
    // If there is already a faucet key file, then open it and parse the keypair.  If there
    // isn't one, then create a temp directory and generate the keypair
    if let Some(faucet_account_file) = file_path {
        match load_key_from_file(faucet_account_file.clone()) {
            Ok(keypair) => (keypair, faucet_account_file.to_string(), None),
            Err(e) => {
                panic!(
                    "Unable to read faucet account file: {}, {}",
                    faucet_account_file, e
                );
            }
        }
    } else {
        // Generate keypair in temp directory
        let tmp_dir =
            TempDir::new("keypair").expect("Unable to create temp dir for faucet keypair");
        let faucet_key_file_path = tmp_dir
            .path()
            .join("temp_faucet_keys")
            .to_str()
            .unwrap()
            .to_string();
        (
            crate::create_faucet_key_file(&faucet_key_file_path),
            faucet_key_file_path,
            Some(tmp_dir),
        )
    }
}
