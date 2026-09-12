//! Typed RPC family dispatch over narrow operation ports.

mod beacon;
mod kex;
mod kv;
mod logsend;
mod probe;
mod realtime;
mod registration;
mod team;
mod user;

use foks_rpc::RpcStatus;

use crate::auth::Principal;
use crate::rpc::{RouteId, RoutedCall};

use super::ServerData;

pub(super) fn response(
    data: &ServerData,
    call: RoutedCall,
    principal: Option<&Principal>,
) -> Result<Vec<u8>, RpcStatus> {
    use RouteId::*;

    match call.route.id {
        ProbeProbe
        | MerkleQueryLookup
        | MerkleQueryGetHistoricalRoots
        | MerkleQueryGetCurrentRoot
        | MerkleQueryGetCurrentRootHash
        | MerkleQueryCheckKeyExists
        | MerkleQueryGetCurrentRootSigned
        | MerkleQueryMLookup
        | MerkleQuerySelectVHost
        | RegSelectVHost
        | KvStoreSelectVHost => probe::response(data, call),
        BeaconBeaconLookup => beacon::response(data, call),
        RegReserveUsername
        | RegCheckNameExists
        | RegJoinWaitList
        | RegResolveUsername
        | RegProbeKeyExists
        | RegGetClientVersionInfo
        | RegGetServerConfig
        | RegGetLoginChallenge
        | RegLogin
        | RegStretchVersion
        | RegGetClientCertChain
        | RegSignup
        | RegCheckInviteCode
        | RegGetSubkeyBoxChallenge
        | RegLoadSubkeyBox
        | RegGetUIDLookupChallege
        | RegLookupUIDByDevice
        | RegLoadUserChain => registration::response(data, call),
        UserResolveUsername
        | UserPing
        | UserGetDeviceNag
        | UserClearDeviceNag
        | UserSetPassphrase
        | UserChangePassphrase
        | UserGetSalt
        | UserNextPassphraseGeneration
        | UserStretchVersion
        | UserGetPpeParcel
        | UserProvisionDevice
        | UserRevokeDevice
        | UserLoadUserChain
        | UserGetPukForRole
        | UserPutYubiManagementKey
        | UserGetYubiManagementKey
        | UserGetAllYubiManagementKeys
        | UserGetHostConfig
        | UserPostGenericLink
        | UserLoadGenericChain
        | UserGetTeamListServerTrust
        | UserGrantRemoteViewPermissionForUser => user::response(data, call, principal),
        TeamLoaderGetTeamVOBearerTokenChallenge
        | TeamLoaderActivateTeamVOBearerToken
        | TeamLoaderCheckTeamVOBearerToken
        | TeamLoaderLoadTeamChain
        | TeamLoaderLoadTeamMembershipChain
        | TeamLoaderLoadRemovalForMember
        | TeamLoaderLoadTeamRemoteViewTokens
        | TeamLoaderGetServerConfig
        | TeamMemberGrantRemoteViewPermissionForTeam
        | TeamAdminReserveTeamname
        | TeamAdminCreateTeam
        | TeamAdminEditTeam
        | TeamAdminMakeInertTeamBearerToken
        | TeamAdminActivateTeamBearerToken
        | TeamAdminCheckTeamBearerToken
        | TeamAdminLoadRemovalKeyBoxForTeamAdmin
        | TeamAdminPostTeamMembershipLink
        | TeamAdminGetTeamConfig
        | TeamAdminCreateTeamAdHoc => team::response(data, call, principal),
        KvStoreMkdir
        | KvStorePut
        | KvStorePutRoot
        | KvStoreFileUploadInit
        | KvStoreFileUploadChunk
        | KvStorePutSmallFileOrSymlink
        | KvStoreGetRoot
        | KvStoreGet
        | KvStoreGetNode
        | KvStoreGetEncryptedChunk
        | KvStoreGetDir
        | KvStoreCacheCheck
        | KvStoreList
        | KvStoreLockAcquire
        | KvStoreLockRelease
        | KvStoreUsage => kv::response(data, call, principal),
        KexSend | KexReceive => kex::response(data, call),
        LogSendLogSendInit | LogSendLogSendInitFile | LogSendLogSendUploadBlock => {
            logsend::response(data, call, principal)
        }
        RealTimeFennecChatCapabilities
        | RealTimeRtNewChannel
        | RealTimeRtListAllChannelsForTeam
        | RealTimeRtSend
        | RealTimeRtGetThread
        | RealTimeRtGetInboxVersion
        | RealTimeRtGetChangedThreads
        | RealTimeRtReadThrough
        | RealTimeRtPollInbox
        | RealTimeRtSelectVHost
        | RealTimeRtGetThreadRecents => realtime::response(data, call, principal),
        RealTimeRtGetChannel
        | TeamAdminPutTeamCert
        | TeamAdminGetCurrentTeamCerts
        | TeamAdminLoadTeamRemoteJoinReq
        | TeamAdminPostTeamRemoval
        | TeamAdminLoadTeamRawInbox
        | TeamAdminRejectJoinReq => Err(RpcStatus::Unsupported),
    }
}

pub(super) async fn kex_receive_response(
    data: &ServerData,
    call: RoutedCall,
) -> Result<Vec<u8>, RpcStatus> {
    kex::receive_response(data, call).await
}

pub(super) async fn realtime_poll_response(
    data: &ServerData,
    call: RoutedCall,
    certificate: Vec<u8>,
) -> Result<Vec<u8>, RpcStatus> {
    realtime::poll_response(data, call, certificate).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn executable_dispatch_does_not_compare_protocol_or_method_names() {
        let dispatch_sources = [
            include_str!("mod.rs"),
            include_str!("kv.rs"),
            include_str!("realtime.rs"),
            include_str!("probe.rs"),
            include_str!("registration.rs"),
            include_str!("team.rs"),
            include_str!("user.rs"),
            include_str!("../../../services/kv.rs"),
        ];
        let protocol_comparison = ["route.", "protocol =="].concat();
        let method_comparison = ["route.", "method =="].concat();
        let method_match = ["match ", "method"].concat();
        for source in dispatch_sources {
            assert!(!source.contains(&protocol_comparison));
            assert!(!source.contains(&method_comparison));
            assert!(!source.contains(&method_match));
        }
    }
}
