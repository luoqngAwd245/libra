// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! The following is a minimalist version of a hierarchical key derivation library for the
//! LibraWallet.
//!
//! Note that the Libra Blockchain makes use of ed25519 Edwards Digital Signature Algorithm
//! (EdDSA) and therefore, BIP32 Public Key derivation is not available without falling back to
//! a non-deterministic Schnorr signature scheme. As LibraWallet is meant to be a minimalist
//! reference implementation of a simple wallet, the following does not deviate from the
//! ed25519 spec. In a future iteration of this wallet, we will also provide an implementation
//! of a Schnorr variant over curve25519 and demonstrate our proposal for BIP32-like public key
//! derivation.
//!
//! Note further that the Key Derivation Function (KDF) chosen in the derivation of Child
//! Private Keys adheres to [HKDF RFC 5869](https://tools.ietf.org/html/rfc5869).
//!
//! 以下是LibraWallet的层次结构密钥派生库的简约版本。
//!
//!请注意，天秤座区块链使用ed25519爱德华兹数字签名算法（EdDSA），因此，如果不使用不确定的Schnorr签名方案，
//! 则无法使用BIP32公钥。 由于LibraWallet旨在成为极简主义者参考简单钱包的实现，以下内容不偏离ed25519规范。
//! 在此钱包的将来版本中，我们还将在curve25519上提供Schnorr变体的实现，并演示我们对类似于BIP32的公钥的建议
//! 推导。
//!
//!还要注意，在派生子私钥时选择的密钥推导函数（KDF）遵循[HKDF RFC 5869](https://tools.ietf.org/html/rfc5869).

use byteorder::{ByteOrder, LittleEndian};
use crypto::{hmac::Hmac as CryptoHmac, pbkdf2::pbkdf2, sha3::Sha3};
use ed25519_dalek;
use libra_crypto::{hash::HashValue, hkdf::Hkdf};
use serde::{Deserialize, Serialize};
use sha3::Sha3_256;
use std::{convert::TryFrom, ops::AddAssign};
use tiny_keccak::Keccak;
use types::account_address::AccountAddress;

use crate::{error::Result, mnemonic::Mnemonic};

/// Master is a set of raw bytes that are used for child key derivation
/// 主控是一组用于子密钥派生的原始字节
pub struct Master([u8; 32]);
impl_array_newtype!(Master, u8, 32);
impl_array_newtype_show!(Master);
impl_array_newtype_encodable!(Master, u8, 32);

/// A child number for a derived key, used to derive a certain private key from the Master
/// 派生密钥的子代号，用于从主服务器派生某个私钥
#[derive(Default, Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ChildNumber(pub(crate) u64);

impl ChildNumber {
    /// Constructor from u64
    pub fn new(child_number: u64) -> Self {
        Self(child_number)
    }

    /// Bump the ChildNumber
    pub fn increment(&mut self) {
        self.add_assign(Self(1));
    }
}

impl std::ops::AddAssign for ChildNumber {
    fn add_assign(&mut self, other: Self) {
        *self = Self(self.0 + other.0)
    }
}

impl std::convert::AsRef<u64> for ChildNumber {
    fn as_ref(&self) -> &u64 {
        &self.0
    }
}

impl std::convert::AsMut<u64> for ChildNumber {
    fn as_mut(&mut self) -> &mut u64 {
        &mut self.0
    }
}

/// Derived private key.
/// 派生私钥
pub struct ExtendedPrivKey {
    /// Child number of the key used to derive from Parent.
    _child_number: ChildNumber,
    /// Private key.
    private_key: ed25519_dalek::SecretKey,
}

impl ExtendedPrivKey {
    /// Constructor for creating an ExtendedPrivKey from a ed25519 PrivateKey. Note that the
    /// ChildNumber are not used in this iteration of LibraWallet, but in order to
    /// enable more general Hierarchical KeyDerivation schemes, we include it for completeness.
    /// 用于从ed25519私钥创建ExtendedPrivKey的构造方法。 请注意，在此LibraWallet迭代中未使用ChildNumber，
    /// 但是为了启用更通用的Hierarchical KeyDerivation方案，为完整起见我们将其包括在内。
    pub fn new(_child_number: ChildNumber, private_key: ed25519_dalek::SecretKey) -> Self {
        Self {
            _child_number,
            private_key,
        }
    }

    /// Returns the PublicKey associated to a particular ExtendedPrivKey
    /// 返回与特定ExtendedPrivKey关联的PublicKey
    pub fn get_public(&self) -> ed25519_dalek::PublicKey {
        (&self.private_key).into()
    }

    /// Computes the sha3 hash of the PublicKey and attempts to construct a Libra AccountAddress
    /// from the raw bytes of the pubkey hash
    /// 计算PublicKey的sha3哈希，并尝试从pubkey哈希的原始字节构造Libra AccountAddress
    pub fn get_address(&self) -> Result<AccountAddress> {
        let public_key = self.get_public();
        let mut keccak = Keccak::new_sha3_256();
        let mut hash = [0u8; 32];
        keccak.update(&public_key.to_bytes());
        keccak.finalize(&mut hash);
        let addr = AccountAddress::try_from(&hash[..])?;
        Ok(addr)
    }

    /// Libra specific sign function that is capable of signing an arbitrary HashValue
    /// NOTE: In Libra, we do not sign the raw bytes of a transaction, instead we sign the raw
    /// bytes of the sha3 hash of the raw bytes of a transaction. It is important to note that the
    /// raw bytes of the sha3 hash will be hashed again as part of the ed25519 signature algorithm.
    /// In other words: In Libra, the message used for signature and verification is the sha3 hash
    /// of the transaction. This sha3 hash is then hashed again using SHA512 to arrive at the
    /// deterministic nonce for the EdDSA.
    /// libra特定的签名功能，能够对任意HashValue进行签名
    ///  注意：在Libra中，我们不对事务的原始字节进行签名，而是对事务的原始字节的sha3哈希的原始字节进行签名。
    /// 重要的是要注意，sha3哈希的原始字节将作为ed25519签名算法的一部分再次被哈希。
    /// 换句话说：在Libra中，用于签名和验证的消息是事务的sha3哈希。 然后使用SHA512再次对该sha3哈希
    /// 进行哈希处理，以得出EdDSA的确定性随机数。
    pub fn sign(&self, msg: HashValue) -> ed25519_dalek::Signature {
        let public_key: ed25519_dalek::PublicKey = (&self.private_key).into();
        let expanded_secret_key: ed25519_dalek::ExpandedSecretKey =
            ed25519_dalek::ExpandedSecretKey::from(&self.private_key);
        expanded_secret_key.sign(msg.as_ref(), &public_key)
    }
}

/// Wrapper struct from which we derive child keys
pub struct KeyFactory {
    master: Master,
}

impl KeyFactory {
    const MNEMONIC_SALT_PREFIX: &'static [u8] = b"LIBRA WALLET: mnemonic salt prefix$";
    const MASTER_KEY_SALT: &'static [u8] = b"LIBRA WALLET: master key salt$";
    const INFO_PREFIX: &'static [u8] = b"LIBRA WALLET: derived key$";
    /// Instantiate a new KeyFactor from a Seed, where the [u8; 64] raw bytes of the Seed are used
    /// to derive both the Master
    /// 从种子实例化一个新的KeyFactor，其中[u8; 64] Seed的原始字节用于派生两个Master
    pub fn new(seed: &Seed) -> Result<Self> {
        let hkdf_extract = Hkdf::<Sha3_256>::extract(Some(KeyFactory::MASTER_KEY_SALT), &seed.0)?;

        Ok(Self {
            master: Master::from(&hkdf_extract[..32]),
        })
    }

    /// Getter for the Master
    pub fn master(&self) -> &[u8] {
        &self.master.0[..]
    }

    /// Derive a particular PrivateKey at a certain ChildNumber
    ///
    /// Note that the function below  adheres to [HKDF RFC 5869](https://tools.ietf.org/html/rfc5869).
    pub fn private_child(&self, child: ChildNumber) -> Result<ExtendedPrivKey> {
        // application info in the HKDF context is defined as Libra derived key$child_number.
        let mut le_n = [0u8; 8];
        LittleEndian::write_u64(&mut le_n, child.0);
        let mut info = KeyFactory::INFO_PREFIX.to_vec();
        info.extend_from_slice(&le_n);

        let hkdf_expand = Hkdf::<Sha3_256>::expand(&self.master(), Some(&info), 32)?;
        let sk = ed25519_dalek::SecretKey::from_bytes(&hkdf_expand)?;

        Ok(ExtendedPrivKey::new(child, sk))
    }
}

/// Seed is the output of a one-way function, which accepts a Mnemonic as input
/// 种子是单向函数的输出，该函数接受助记符作为输入
pub struct Seed([u8; 32]);

impl Seed {
    /// Get the underlying Seed internal data
    pub fn data(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl Seed {
    /// This constructor implements the one-way function that allows to generate a Seed from a
    /// particular Mnemonic and salt. WalletLibrary implements a fixed salt, but a user could
    /// choose a user-defined salt instead of the hardcoded one.
    /// 该构造函数实现单向功能，该功能允许从特定的助记符和盐生成种子。 WalletLibrary实现了固定的盐，
    /// 但是用户可以选择用户定义的盐，而不是硬编码的盐。
    pub fn new(mnemonic: &Mnemonic, salt: &str) -> Seed {
        let mut mac = CryptoHmac::new(Sha3::sha3_256(), mnemonic.to_string().as_bytes());
        let mut output = [0u8; 32];

        let mut msalt = KeyFactory::MNEMONIC_SALT_PREFIX.to_vec();
        msalt.extend_from_slice(salt.as_bytes());

        pbkdf2(&mut mac, &msalt, 2048, &mut output);
        Seed(output)
    }
}

#[test]
fn assert_default_child_number() {
    assert_eq!(ChildNumber::default(), ChildNumber(0));
}

#[test]
fn test_key_derivation() {
    let data = hex::decode("7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f").unwrap();
    let mnemonic = Mnemonic::from("legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal will").unwrap();
    assert_eq!(
        mnemonic.to_string(),
        Mnemonic::mnemonic(&data).unwrap().to_string()
    );
    let seed = Seed::new(&mnemonic, "LIBRA");

    let key_factory = KeyFactory::new(&seed).unwrap();
    assert_eq!(
        "16274c9618ed59177ca948529c1884ba65c57984d562ec2b4e5aa1ee3e3903be",
        hex::encode(&key_factory.master())
    );

    // Check child_0 key derivation.
    let child_private_0 = key_factory.private_child(ChildNumber(0)).unwrap();
    assert_eq!(
        "358a375f36d74c30b7f3299b62d712b307725938f8cc931100fbd10a434fc8b9",
        hex::encode(&child_private_0.private_key.to_bytes()[..])
    );

    // Check determinism, regenerate child_0.
    let child_private_0_again = key_factory.private_child(ChildNumber(0)).unwrap();
    assert_eq!(
        hex::encode(&child_private_0.private_key.to_bytes()[..]),
        hex::encode(&child_private_0_again.private_key.to_bytes()[..])
    );

    // Check child_1 key derivation.
    let child_private_1 = key_factory.private_child(ChildNumber(1)).unwrap();
    assert_eq!(
        "a325fe7d27b1b49f191cc03525951fec41b6ffa2d4b3007bb1d9dd353b7e56a6",
        hex::encode(&child_private_1.private_key.to_bytes()[..])
    );

    let mut child_1_again = ChildNumber(0);
    child_1_again.increment();
    assert_eq!(ChildNumber(1), child_1_again);

    // Check determinism, regenerate child_1, but by incrementing ChildNumber(0).
    let child_private_1_from_increment = key_factory.private_child(child_1_again).unwrap();
    assert_eq!(
        "a325fe7d27b1b49f191cc03525951fec41b6ffa2d4b3007bb1d9dd353b7e56a6",
        hex::encode(&child_private_1_from_increment.private_key.to_bytes()[..])
    );
}
