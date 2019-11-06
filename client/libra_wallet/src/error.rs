// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use failure;
use libra_crypto::hkdf::HkdfError;
use std::{convert, error::Error, fmt, io};

/// We define our own Result type in order to not have to import the libra/common/failure_ext
/// 为了不importlibra/common/failure_ext，我们定义自己所有的返回类型
pub type Result<T> = ::std::result::Result<T, WalletError>;

/// Libra Wallet Error is a convenience enum for generating arbitrary WalletErrors. Currently, only
/// the LibraWalletGeneric error is being used, but there are plans to add more specific errors as
/// LibraWallet matures
/// 天秤座钱包错误是用于生成任意WalletErrors的便捷枚举。 当前，仅使用LibraWalletGeneric错误，但是随着
/// LibraWallet的成熟，有计划添加更多特定错误。
pub enum WalletError {
    /// generic error message
    /// 通用错误信息
    LibraWalletGeneric(String),
}

impl Error for WalletError {
    fn description(&self) -> &str {
        match *self {
            WalletError::LibraWalletGeneric(ref s) => s,
        }
    }

    fn cause(&self) -> Option<&dyn Error> {
        match *self {
            WalletError::LibraWalletGeneric(_) => None,
        }
    }
}

impl fmt::Display for WalletError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            WalletError::LibraWalletGeneric(ref s) => write!(f, "LibraWalletGeneric: {}", s),
        }
    }
}

impl fmt::Debug for WalletError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        (self as &dyn fmt::Display).fmt(f)
    }
}

impl convert::From<WalletError> for io::Error {
    fn from(_err: WalletError) -> io::Error {
        match _err {
            WalletError::LibraWalletGeneric(s) => io::Error::new(io::ErrorKind::Other, s),
        }
    }
}

impl convert::From<io::Error> for WalletError {
    fn from(err: io::Error) -> WalletError {
        WalletError::LibraWalletGeneric(err.description().to_string())
    }
}

impl convert::From<failure::prelude::Error> for WalletError {
    fn from(err: failure::prelude::Error) -> WalletError {
        WalletError::LibraWalletGeneric(format!("{}", err))
    }
}

impl convert::From<protobuf::error::ProtobufError> for WalletError {
    fn from(err: protobuf::error::ProtobufError) -> WalletError {
        WalletError::LibraWalletGeneric(err.description().to_string())
    }
}

impl convert::From<ed25519_dalek::SignatureError> for WalletError {
    fn from(err: ed25519_dalek::SignatureError) -> WalletError {
        WalletError::LibraWalletGeneric(format!("{}", err))
    }
}

impl convert::From<HkdfError> for WalletError {
    fn from(err: HkdfError) -> WalletError {
        WalletError::LibraWalletGeneric(format!("{}", err))
    }
}
