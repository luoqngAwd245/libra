use crate::chained_bft::consensus_types::{
    quorum_cert::QuorumCert, timeout_msg::PacemakerTimeoutCertificate,
};
use network;
use proto_conv::{FromProto, IntoProto};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
/// This struct describes basic synchronization metadata.
/// 此结构描述基本同步元数据。
pub struct SyncInfo {
    /// Highest quorum certificate known to the peer.
    /// 对等方已知的最高仲裁证书。
    highest_quorum_cert: QuorumCert,
    /// Highest ledger info known to the peer.
    /// 对等方知道的最高分类帐信息。
    highest_ledger_info: QuorumCert,
    /// Optional highest timeout certificate if available.
    /// 可选的最高超时证书（如果有）。
    highest_timeout_cert: Option<PacemakerTimeoutCertificate>,
}

impl Display for SyncInfo {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let htc_repr = match self.highest_timeout_certificate() {
            Some(tc) => format!("TC for round {}", tc.round()),
            None => "None".to_string(),
        };
        write!(
            f,
            "HQC: {}, HLI: {}, HTC: {}",
            self.highest_quorum_cert, self.highest_ledger_info, htc_repr,
        )
    }
}

impl SyncInfo {
    pub fn new(
        highest_quorum_cert: QuorumCert,
        highest_ledger_info: QuorumCert,
        highest_timeout_cert: Option<PacemakerTimeoutCertificate>,
    ) -> Self {
        Self {
            highest_quorum_cert,
            highest_ledger_info,
            highest_timeout_cert,
        }
    }

    /// Highest quorum certificate
    /// 最高法定证书
    pub fn highest_quorum_cert(&self) -> &QuorumCert {
        &self.highest_quorum_cert
    }

    /// Highest ledger info
    /// 最高分类帐信息
    pub fn highest_ledger_info(&self) -> &QuorumCert {
        &self.highest_ledger_info
    }

    /// Highest timeout certificate if available
    /// 最高超时证书（如果有）
    #[allow(dead_code)]
    pub fn highest_timeout_certificate(&self) -> Option<&PacemakerTimeoutCertificate> {
        self.highest_timeout_cert.as_ref()
    }
}

impl FromProto for SyncInfo {
    type ProtoType = network::proto::SyncInfo;

    fn from_proto(mut object: network::proto::SyncInfo) -> failure::Result<Self> {
        let highest_quorum_cert = QuorumCert::from_proto(object.take_highest_quorum_cert())?;
        let highest_ledger_info = QuorumCert::from_proto(object.take_highest_ledger_info())?;
        let highest_timeout_cert = if let Some(tc) = object.highest_timeout_cert.into_option() {
            Some(PacemakerTimeoutCertificate::from_proto(tc)?)
        } else {
            None
        };
        Ok(SyncInfo::new(
            highest_quorum_cert,
            highest_ledger_info,
            highest_timeout_cert,
        ))
    }
}
impl IntoProto for SyncInfo {
    type ProtoType = network::proto::SyncInfo;

    fn into_proto(self) -> Self::ProtoType {
        let mut proto = Self::ProtoType::new();
        proto.set_highest_quorum_cert(self.highest_quorum_cert.into_proto());
        proto.set_highest_ledger_info(self.highest_ledger_info.into_proto());
        if let Some(tc) = self.highest_timeout_cert {
            proto.set_highest_timeout_cert(tc.into_proto());
        }
        proto
    }
}
