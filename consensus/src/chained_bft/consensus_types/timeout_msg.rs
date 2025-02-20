// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::chained_bft::{
    common::{Author, Round},
    consensus_types::{
        sync_info::SyncInfo, timeout_msg::PacemakerTimeoutCertificateVerificationError::*,
    },
};
use canonical_serialization::{CanonicalSerialize, CanonicalSerializer, SimpleSerializer};
use crypto::{
    hash::{CryptoHash, CryptoHasher, PacemakerTimeoutHasher, TimeoutMsgHasher},
    HashValue,
};
use network;
use nextgen_crypto::ed25519::*;
use proto_conv::{FromProto, IntoProto};
use protobuf::RepeatedField;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, convert::TryFrom, fmt, iter::FromIterator};
use types::{
    account_address::AccountAddress,
    validator_signer::ValidatorSigner,
    validator_verifier::{ValidatorVerifier, VerifyError},
};

// Internal use only. Contains all the fields in PaceMakerTimeout that contributes to the
// computation of its hash.
// 限内部使用。 包含PaceMakerTimeout中有助于其哈希计算的所有字段。
struct PacemakerTimeoutSerializer {
    round: Round,
    author: Author,
}

impl CanonicalSerialize for PacemakerTimeoutSerializer {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> failure::Result<()> {
        serializer.encode_u64(self.round)?;
        serializer.encode_struct(&self.author)?;
        Ok(())
    }
}

impl CryptoHash for PacemakerTimeoutSerializer {
    type Hasher = PacemakerTimeoutHasher;

    fn hash(&self) -> HashValue {
        let mut state = Self::Hasher::default();
        state.write(&SimpleSerializer::<Vec<u8>>::serialize(self).expect("Should serialize."));
        state.finish()
    }
}

/// This message will be broadcast by a pacemaker as part of TimeoutMsg when its local
/// timeout for a round is reached.  Once f+1 PacemakerTimeout structs
/// from unique authors is gathered it forms a TimeoutCertificate.  A TimeoutCertificate is
/// a proof that will cause a replica to advance to the minimum round in the TimeoutCertificate.
/// 当达到当地的一轮超时时间时，起搏器将将该消息作为TimeoutMsg的一部分进行广播。 收集到来自唯一作者的f + 1
/// PacemakerTimeout结构后，它将形成TimeoutCertificate。 TimeoutCertificate是一种证明，它将使副本前进到
/// TimeoutCertificate中的最小回合。
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct PacemakerTimeout {
    round: Round,
    author: Author,
    signature: Ed25519Signature,
}

impl PacemakerTimeout {
    /// Creates new PacemakerTimeout
    pub fn new(round: Round, validator_signer: &ValidatorSigner<Ed25519PrivateKey>) -> Self {
        let author = validator_signer.author();
        let digest = PacemakerTimeoutSerializer { round, author }.hash();
        let signature = validator_signer
            .sign_message(digest)
            .expect("Failed to sign PacemakerTimeout");
        PacemakerTimeout {
            round,
            author,
            signature,
        }
    }

    fn pacemaker_timeout_digest(author: AccountAddress, round: Round) -> HashValue {
        PacemakerTimeoutSerializer { round, author }.hash()
    }

    /// Calculates digest for this struct
    /// 计算该结构的摘要
    pub fn digest(&self) -> HashValue {
        Self::pacemaker_timeout_digest(self.author, self.round)
    }

    pub fn round(&self) -> Round {
        self.round
    }

    /// Verifies that this message has valid signature
    /// 验证此消息具有有效签名
    pub fn verify(
        &self,
        validator: &ValidatorVerifier<Ed25519PublicKey>,
    ) -> Result<(), VerifyError> {
        validator.verify_signature(self.author, self.digest(), &self.signature)
    }

    /// Returns the author of the timeout
    /// 返回超时的作者
    pub fn author(&self) -> Author {
        self.author
    }

    /// Returns the signature of the author for this timeout
    pub fn signature(&self) -> &Ed25519Signature {
        &self.signature
    }
}

impl IntoProto for PacemakerTimeout {
    type ProtoType = network::proto::PacemakerTimeout;

    fn into_proto(self) -> Self::ProtoType {
        let mut proto = Self::ProtoType::new();
        proto.set_round(self.round);
        proto.set_author(self.author.into());
        proto.set_signature(self.signature.to_bytes().as_ref().into());
        proto
    }
}

impl FromProto for PacemakerTimeout {
    type ProtoType = network::proto::PacemakerTimeout;

    fn from_proto(mut object: Self::ProtoType) -> failure::Result<Self> {
        let round = object.get_round();
        let author = Author::try_from(object.take_author())?;
        let signature = Ed25519Signature::try_from(object.get_signature())?;
        Ok(PacemakerTimeout {
            round,
            author,
            signature,
        })
    }
}

// Internal use only. Contains all the fields in TimeoutMsg that contributes to the computation of
// its hash.
// 限内部使用。 包含TimeoutMsg中有助于其哈希计算的所有字段。
struct TimeoutMsgSerializer {
    pacemaker_timeout_digest: HashValue,
}

impl CanonicalSerialize for TimeoutMsgSerializer {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> failure::Result<()> {
        serializer.encode_raw_bytes(self.pacemaker_timeout_digest.as_ref())?;
        Ok(())
    }
}

impl CryptoHash for TimeoutMsgSerializer {
    type Hasher = TimeoutMsgHasher;

    fn hash(&self) -> HashValue {
        let mut state = Self::Hasher::default();
        state.write(&SimpleSerializer::<Vec<u8>>::serialize(self).expect("Should serialize."));
        state.finish()
    }
}

/// This message will be broadcast by a pacemaker when its local timeout for a round is reached.
/// Once the broadcasts start, retries will continue for every timeout until the round changes.
/// Retries are required since, say if a proposer for a round r was unresponsive, it might not
/// propose if it misses even only one PacemakerTimeoutMsg.
///
/// The expected proposer will wait until n-f such messages are received before proposing to
/// ensure liveness (a next proposal has the highest quorum certificate across all replicas
/// as justification). If the expected proposer has a quorum certificate on round r-1, it need
/// not wait until n-f such messages are received and can make a proposal justified
/// by this quorum certificate.
/// 当达到起搏器的本地超时时间时，该消息将由起搏器广播。 广播开始后，每次超时都会重试，直到回合更改为止。
/// 需要重试，因为，例如，如果一轮回合的提议者没有响应，则它可能不会提议它是否仅错过一个PacemakerTimeoutMsg。
///
///预期的提议者将等到收到n-f个这样的消息，然后提出提议以确保活动性（下一个提议具有所有副本中最高的仲裁证书作为理由）。
/// 如果预期的提议者在第r-1回合上具有法定证书，则不必等到收到n-f个这样的消息，即可提出由该法定证书证明合理的提议。
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct TimeoutMsg {
    sync_info: SyncInfo,
    pacemaker_timeout: PacemakerTimeout,
    signature: Ed25519Signature,
}

impl TimeoutMsg {
    /// Creates new TimeoutMsg
    pub fn new(
        sync_info: SyncInfo,
        pacemaker_timeout: PacemakerTimeout,
        validator_signer: &ValidatorSigner<Ed25519PrivateKey>,
    ) -> TimeoutMsg {
        let digest = Self::new_round_digest(pacemaker_timeout.digest());
        let signature = validator_signer
            .sign_message(digest)
            .expect("Failed to sign PacemakerTimeoutMsg");
        TimeoutMsg {
            sync_info,
            pacemaker_timeout,
            signature,
        }
    }

    fn new_round_digest(pacemaker_timeout_digest: HashValue) -> HashValue {
        TimeoutMsgSerializer {
            pacemaker_timeout_digest,
        }
        .hash()
    }

    /// Calculates digest for this message
    pub fn digest(&self) -> HashValue {
        Self::new_round_digest(self.pacemaker_timeout.digest())
    }

    /// SyncInfo of the given timeout message
    pub fn sync_info(&self) -> &SyncInfo {
        &self.sync_info
    }

    /// Returns a reference to the included PacemakerTimeout
    pub fn pacemaker_timeout(&self) -> &PacemakerTimeout {
        &self.pacemaker_timeout
    }

    /// Verifies that this message has valid signature
    pub fn verify(
        &self,
        validator: &ValidatorVerifier<Ed25519PublicKey>,
    ) -> Result<(), VerifyError> {
        validator.verify_signature(self.author(), self.digest(), &self.signature)?;
        self.pacemaker_timeout.verify(validator)
    }

    /// Returns the author of the TimeoutMsg
    pub fn author(&self) -> Author {
        self.pacemaker_timeout.author()
    }

    /// Returns a reference to the signature of the author
    #[allow(dead_code)]
    pub fn signature(&self) -> &Ed25519Signature {
        &self.signature
    }
}

impl FromProto for TimeoutMsg {
    type ProtoType = network::proto::TimeoutMsg;

    fn from_proto(mut object: network::proto::TimeoutMsg) -> failure::Result<Self> {
        let sync_info = SyncInfo::from_proto(object.take_sync_info())?;
        let pacemaker_timeout = PacemakerTimeout::from_proto(object.take_pacemaker_timeout())?;
        let signature = Ed25519Signature::try_from(object.get_signature())?;
        Ok(TimeoutMsg {
            sync_info,
            pacemaker_timeout,
            signature,
        })
    }
}

impl IntoProto for TimeoutMsg {
    type ProtoType = network::proto::TimeoutMsg;

    fn into_proto(self) -> Self::ProtoType {
        let mut proto = Self::ProtoType::new();
        proto.set_sync_info(self.sync_info.into_proto());
        proto.set_pacemaker_timeout(self.pacemaker_timeout.into_proto());
        proto.set_signature(self.signature.to_bytes().as_ref().into());
        proto
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
/// Proposal can include this timeout certificate as justification for switching to next round
/// 提案可以将此超时证书作为切换至下一轮的理由
pub struct PacemakerTimeoutCertificate {
    round: Round,
    timeouts: Vec<PacemakerTimeout>,
}

/// PacemakerTimeoutCertificate verification errors.
/// PacemakerTimeoutCertificate验证错误。
#[derive(Debug, PartialEq)]
pub enum PacemakerTimeoutCertificateVerificationError {
    /// Number of signed timeouts is less then required quorum size
    /// 签名超时数小于所需的仲裁大小
    NoQuorum,
    /// Round in message does not match calculated rounds based on signed timeouts
    /// 舍入消息与基于签名超时计算出的舍入不匹配
    RoundMismatch { expected: Round },
    /// The signature on one of timeouts doesn't pass verification
    SigVerifyError(Author, VerifyError),
}

impl fmt::Display for PacemakerTimeoutCertificate {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "TimeoutCertificate[round: {}, timeouts:[", self.round)?;
        for (idx, timeout) in self.timeouts.iter().enumerate() {
            write!(f, "<{}>", timeout.round())?;
            if idx != self.timeouts.len() - 1 {
                write!(f, ", ")?;
            }
        }
        write!(f, "]")
    }
}

impl PacemakerTimeoutCertificate {
    /// Creates new PacemakerTimeoutCertificate
    pub fn new(round: Round, timeouts: Vec<PacemakerTimeout>) -> PacemakerTimeoutCertificate {
        PacemakerTimeoutCertificate { round, timeouts }
    }

    /// Verifies that timeouts in message actually certify the round
    pub fn verify(
        &self,
        validator: &ValidatorVerifier<Ed25519PublicKey>,
    ) -> Result<(), PacemakerTimeoutCertificateVerificationError> {
        let mut min_round: Option<Round> = None;
        let mut unique_authors = HashSet::new();
        for timeout in &self.timeouts {
            if let Err(e) =
                validator.verify_signature(timeout.author(), timeout.digest(), timeout.signature())
            {
                return Err(SigVerifyError(timeout.author(), e));
            }
            unique_authors.insert(timeout.author());
            let timeout_round = timeout.round();
            min_round = Some(min_round.map_or(timeout_round, move |x| x.min(timeout_round)))
        }
        if unique_authors.len() < validator.quorum_size() {
            return Err(NoQuorum);
        }
        if min_round == Some(self.round) {
            Ok(())
        } else {
            Err(RoundMismatch {
                expected: min_round.unwrap_or(0),
            })
        }
    }

    /// Returns the round of the timeout
    /// 返回超时时间
    pub fn round(&self) -> Round {
        self.round
    }

    /// Returns the timeouts that certify the PacemakerTimeoutCertificate
    /// 返回证明PacemakerTimeoutCertificate的超时
    #[allow(dead_code)]
    pub fn timeouts(&self) -> &Vec<PacemakerTimeout> {
        &self.timeouts
    }
}

impl IntoProto for PacemakerTimeoutCertificate {
    type ProtoType = network::proto::PacemakerTimeoutCertificate;

    fn into_proto(self) -> Self::ProtoType {
        let mut proto = Self::ProtoType::new();
        proto.set_timeouts(RepeatedField::from_iter(
            self.timeouts.into_iter().map(PacemakerTimeout::into_proto),
        ));
        proto.set_round(self.round);
        proto
    }
}

impl FromProto for PacemakerTimeoutCertificate {
    type ProtoType = network::proto::PacemakerTimeoutCertificate;

    fn from_proto(mut object: Self::ProtoType) -> failure::Result<Self> {
        let timeouts = object
            .take_timeouts()
            .into_iter()
            .map(PacemakerTimeout::from_proto)
            .collect::<failure::Result<Vec<_>>>()?;
        Ok(PacemakerTimeoutCertificate::new(
            object.get_round(),
            timeouts,
        ))
    }
}
