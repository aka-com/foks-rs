package main

import (
	"bytes"
	"os"
	"path/filepath"
	"testing"

	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/merkle"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
	"github.com/foks-proj/go-snowpack-rpc/rpc"
)

const checkedUserFixtureDirectory = "../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/user"

// TestCheckedUserRPCFixtures exercises generated argument wrappers, not just
// the lower-level field encodings. Set FOKS_UPDATE_RPC_FIXTURES=1 to refresh
// these narrowly scoped frames without replacing the randomized user corpus.
func TestCheckedUserRPCFixtures(t *testing.T) {
	checkNameFrame, err := rpcRequestFrame(
		rem.RegProtocolID,
		10,
		(&rem.CheckNameExistsArg{Name: proto.Name("alice")}).Export(),
	)
	if err != nil {
		t.Fatalf("encode check-name request: %v", err)
	}
	checkRPCFixture(t, "reg-check-name-request.frame", checkNameFrame)
	checkNameNotFound, err := rpcErrorResponseFrame(0, proto.NewStatusWithUserNotFoundError())
	if err != nil {
		t.Fatalf("encode check-name not-found response: %v", err)
	}
	checkRPCFixture(t, "reg-check-name-not-found-response.frame", checkNameNotFound)

	resolveFrame, err := rpcRequestFrame(
		rem.UserProtocolID,
		23,
		(&rem.UserResolveUsernameArg{A: rem.ResolveUsernameArg{
			N:    proto.Name("alice"),
			Auth: rem.NewLoadUserChainAuthWithOpenvhost(),
		}}).Export(),
	)
	if err != nil {
		t.Fatalf("encode resolve-username request: %v", err)
	}
	checkRPCFixture(t, "user-resolve-username-open-request.frame", resolveFrame)

	clientVersion := proto.ClientVersionExt{
		Vers:            proto.SemVer{Major: 0, Minor: 1, Patch: 9},
		LinkerVersion:   "go1.25",
		LinkerPackaging: "test",
	}
	versionFrame, err := rpcRequestFrame(
		rem.RegProtocolID,
		23,
		(&rem.GetClientVersionInfoArg{Me: clientVersion}).Export(),
	)
	if err != nil {
		t.Fatalf("encode client-version request: %v", err)
	}
	checkRPCFixture(t, "reg-client-version-request.frame", versionFrame)

	versionBytes, err := core.EncodeToBytes(&proto.ServerClientVersionInfo{})
	if err != nil {
		t.Fatalf("encode empty server client-version info: %v", err)
	}
	checkRPCFixture(t, "reg-client-version-empty.snowp", versionBytes)

	configFrame, err := rpcRequestFrame(
		rem.RegProtocolID,
		17,
		(&rem.GetServerConfigArg{}).Export(),
	)
	if err != nil {
		t.Fatalf("encode registration server-config request: %v", err)
	}
	checkRPCFixture(t, "reg-server-config-request.frame", configFrame)

	pingFrame, err := rpcRequestFrame(
		rem.UserProtocolID,
		0,
		(&rem.PingArg{}).Export(),
	)
	if err != nil {
		t.Fatalf("encode user ping request: %v", err)
	}
	checkRPCFixture(t, "user-ping-request.frame", pingFrame)

	nagFrame, err := rpcRequestFrame(
		rem.UserProtocolID,
		25,
		(&rem.GetDeviceNagArg{}).Export(),
	)
	if err != nil {
		t.Fatalf("encode device-nag request: %v", err)
	}
	checkRPCFixture(t, "user-device-nag-request.frame", nagFrame)

	clearNagFrame, err := rpcRequestFrame(
		rem.UserProtocolID,
		26,
		(&rem.ClearDeviceNagArg{Cleared: true}).Export(),
	)
	if err != nil {
		t.Fatalf("encode clear-device-nag request: %v", err)
	}
	checkRPCFixture(t, "user-clear-device-nag-request.frame", clearNagFrame)

	nagBytes, err := core.EncodeToBytes(&proto.DeviceNagInfo{NumDevices: 1})
	if err != nil {
		t.Fatalf("encode device-nag response: %v", err)
	}
	checkRPCFixture(t, "user-device-nag-one.snowp", nagBytes)

	configBytes, err := core.EncodeToBytes(&proto.RegServerConfig{
		Typ: proto.HostType_Standalone,
		View: proto.HostViewership{
			User: proto.ViewershipMode_Open,
			Team: proto.ViewershipMode_Open,
		},
		Icr: proto.InviteCodeRegime_CodeOptional,
	})
	if err != nil {
		t.Fatalf("encode registration server config: %v", err)
	}
	checkRPCFixture(t, "reg-server-config-open.snowp", configBytes)

	teamCreateResponse, err := rpcVoidResponseFrame(rem.TeamAdminProtocolID, 1, 0)
	if err != nil {
		t.Fatalf("encode headerless team-create response: %v", err)
	}
	checkRPCFixture(t, "team-create-response.frame", teamCreateResponse)

	var chain rem.TeamChain
	chainBytes, err := os.ReadFile(filepath.Join(checkedUserFixtureDirectory, "team-chain.snowp"))
	if err != nil {
		t.Fatal(err)
	}
	if err := core.DecodeFromBytes(&chain, chainBytes); err != nil {
		t.Fatalf("decode checked team chain: %v", err)
	}
	if len(chain.Links) == 0 {
		t.Fatal("checked team chain is empty")
	}
	change, _, err := core.OpenGroupChange(&chain.Links[0])
	if err != nil {
		t.Fatalf("open checked team eldest: %v", err)
	}
	hostID := change.Entity.Host
	var checkedPaths proto.MerklePathsCompressed
	checkedPathsBytes, err := os.ReadFile(filepath.Join(checkedUserFixtureDirectory, "user-merkle-paths.snowp"))
	if err != nil {
		t.Fatal(err)
	}
	if err := core.DecodeFromBytes(&checkedPaths, checkedPathsBytes); err != nil {
		t.Fatalf("decode checked Merkle paths: %v", err)
	}
	if len(checkedPaths.Paths) < 2 {
		t.Fatal("checked Merkle paths need at least two entries")
	}
	lookupKey := proto.MerkleTreeRFOutput{}
	for index := range lookupKey {
		lookupKey[index] = byte(index + 1)
	}
	lookupEpoch := proto.MerkleEpno(996)
	lookupFrame, err := rpcRequestFrame(
		rem.MerkleQueryProtocolID,
		0,
		(&rem.MerkleLookupArg{HostID: &hostID, Key: lookupKey, Signed: true, Root: &lookupEpoch}).Export(),
	)
	if err != nil {
		t.Fatalf("encode Merkle lookup request: %v", err)
	}
	checkRPCFixture(t, "merkle-lookup-request.frame", lookupFrame)
	lookupResponse := checkedPaths.Select(0)
	lookupResponseBytes, err := core.EncodeToBytes(&lookupResponse)
	if err != nil {
		t.Fatalf("encode Merkle lookup response: %v", err)
	}
	checkRPCFixture(t, "merkle-lookup-response.snowp", lookupResponseBytes)

	rootHashFrame, err := rpcRequestFrame(
		rem.MerkleQueryProtocolID,
		3,
		(&rem.GetCurrentRootHashArg{HostID: &hostID}).Export(),
	)
	if err != nil {
		t.Fatalf("encode current-root-hash request: %v", err)
	}
	checkRPCFixture(t, "merkle-current-root-hash-request.frame", rootHashFrame)
	var checkedRootHash proto.MerkleRootHash
	if err := merkle.HashRoot(&checkedPaths.Root, &checkedRootHash); err != nil {
		t.Fatalf("hash checked Merkle root: %v", err)
	}
	rootHashResponse := proto.TreeRoot{Epno: checkedPaths.Root.V1().Epno, Hash: checkedRootHash}
	rootHashResponseBytes, err := core.EncodeToBytes(&rootHashResponse)
	if err != nil {
		t.Fatalf("encode current-root-hash response: %v", err)
	}
	checkRPCFixture(t, "merkle-current-root-hash-response.snowp", rootHashResponseBytes)

	checkFrame, err := rpcRequestFrame(
		rem.MerkleQueryProtocolID,
		4,
		(&rem.CheckKeyExistsArg{HostID: &hostID, Key: lookupKey}).Export(),
	)
	if err != nil {
		t.Fatalf("encode Merkle check-key request: %v", err)
	}
	checkRPCFixture(t, "merkle-check-key-request.frame", checkFrame)
	checkResponseBytes, err := core.EncodeToBytes(&rem.MerkleExistsRes{Epno: lookupEpoch, Signed: true})
	if err != nil {
		t.Fatalf("encode Merkle check-key response: %v", err)
	}
	checkRPCFixture(t, "merkle-check-key-response.snowp", checkResponseBytes)
	noRootFrame, err := rpcErrorResponseFrame(0, proto.NewStatusWithMerkleNoRootError())
	if err != nil {
		t.Fatalf("encode Merkle no-root response: %v", err)
	}
	checkRPCFixture(t, "merkle-no-root-response.frame", noRootFrame)
	leafMissingFrame, err := rpcErrorResponseFrame(0, proto.NewStatusWithMerkleLeafNotFoundError())
	if err != nil {
		t.Fatalf("encode Merkle leaf-not-found response: %v", err)
	}
	checkRPCFixture(t, "merkle-leaf-not-found-response.frame", leafMissingFrame)

	multiFrame, err := rpcRequestFrame(
		rem.MerkleQueryProtocolID,
		6,
		(&rem.MerkleMLookupArg{
			HostID: &hostID,
			Keys:   []proto.MerkleTreeRFOutput{lookupKey, {}},
			Signed: false,
			Root:   &lookupEpoch,
		}).Export(),
	)
	if err != nil {
		t.Fatalf("encode Merkle multi-lookup request: %v", err)
	}
	checkRPCFixture(t, "merkle-multi-lookup-request.frame", multiFrame)
	multiResponseBytes, err := core.EncodeToBytes(&checkedPaths)
	if err != nil {
		t.Fatalf("encode Merkle multi-lookup response: %v", err)
	}
	checkRPCFixture(t, "merkle-multi-lookup-response.snowp", multiResponseBytes)

	frame, err := rpcRequestFrameAt(
		rem.MerkleQueryProtocolID,
		2,
		(&rem.GetCurrentRootArg{HostID: &hostID}).Export(),
		1,
	)
	if err != nil {
		t.Fatalf("encode current-root request: %v", err)
	}
	path := filepath.Join(checkedUserFixtureDirectory, "merkle-current-root-request.frame")
	if os.Getenv("FOKS_UPDATE_RPC_FIXTURES") == "1" {
		if err := os.WriteFile(path, frame, 0o644); err != nil {
			t.Fatalf("update current-root request fixture: %v", err)
		}
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(frame, want) {
		t.Fatal("checked current-root request does not use the generated v0.1.9 argument wrapper")
	}

	var uid proto.UID
	uidBytes, err := os.ReadFile(filepath.Join(checkedUserFixtureDirectory, "uid.snowp"))
	if err != nil {
		t.Fatal(err)
	}
	if err := core.DecodeFromBytes(&uid, uidBytes); err != nil {
		t.Fatalf("decode checked user ID: %v", err)
	}
	var deviceID proto.DeviceID
	deviceBytes, err := os.ReadFile(filepath.Join(checkedUserFixtureDirectory, "device-id.snowp"))
	if err != nil {
		t.Fatal(err)
	}
	if err := core.DecodeFromBytes(&deviceID, deviceBytes); err != nil {
		t.Fatalf("decode checked device ID: %v", err)
	}
	var selfToken proto.PermissionToken
	for i := range selfToken {
		selfToken[i] = 0x33
	}
	probeKeyFrame, err := rpcRequestFrame(
		rem.RegProtocolID,
		21,
		(&rem.ProbeKeyExistsArg{Uid: uid, DevID: deviceID, SelfTok: selfToken}).Export(),
	)
	if err != nil {
		t.Fatalf("encode probe-key request: %v", err)
	}
	checkRPCFixture(t, "reg-probe-key-exists-request.frame", probeKeyFrame)
	probeKeyNotFound, err := rpcErrorResponseFrame(0, proto.NewStatusWithKeyNotFoundError("probed key"))
	if err != nil {
		t.Fatalf("encode probe-key not-found response: %v", err)
	}
	checkRPCFixture(t, "reg-probe-key-not-found-response.frame", probeKeyNotFound)
	var teamID proto.TeamID
	teamBytes, err := os.ReadFile(filepath.Join(checkedUserFixtureDirectory, "team-id.snowp"))
	if err != nil {
		t.Fatal(err)
	}
	if err := core.DecodeFromBytes(&teamID, teamBytes); err != nil {
		t.Fatalf("decode checked team ID: %v", err)
	}
	userChainAuthorizations := []struct {
		name     string
		protocol rpc.ProtocolUniqueID
		auth     rem.LoadUserChainAuth
	}{
		{
			name:     "reg-user-load-self-token-request.frame",
			protocol: rem.RegProtocolID,
			auth:     rem.NewLoadUserChainAuthWithSelftoken(selfToken),
		},
		{
			name:     "user-load-local-team-request.frame",
			protocol: rem.UserProtocolID,
			auth:     rem.NewLoadUserChainAuthWithAslocalteam(rem.TeamVOBearerToken(bytes.Repeat([]byte{0x44}, 16))),
		},
		{
			name:     "user-load-open-host-request.frame",
			protocol: rem.UserProtocolID,
			auth:     rem.NewLoadUserChainAuthWithOpenvhost(),
		},
		{
			name:     "user-load-open-or-local-request.frame",
			protocol: rem.UserProtocolID,
			auth:     rem.NewLoadUserChainAuthWithOpenvhostoraslocaluser(),
		},
	}
	for _, fixture := range userChainAuthorizations {
		position := rpc.Position(9)
		if fixture.protocol == rem.RegProtocolID {
			position = 11
		}
		arg := rem.LoadUserChainArg{Uid: uid, Start: proto.ChainEldestSeqno, Auth: fixture.auth}
		var frame []byte
		var err error
		if fixture.protocol == rem.RegProtocolID {
			frame, err = rpcRequestFrame(
				fixture.protocol,
				position,
				(&rem.RegLoadUserChainArg{A: arg}).Export(),
			)
		} else {
			frame, err = rpcRequestFrame(
				fixture.protocol,
				position,
				(&rem.UserLoadUserChainArg{A: arg}).Export(),
			)
		}
		if err != nil {
			t.Fatalf("encode %s: %v", fixture.name, err)
		}
		checkRPCFixture(t, fixture.name, frame)
	}
	parentToken := rem.TeamVOBearerToken(bytes.Repeat([]byte{0x55}, 16))
	parentLoadFrame, err := rpcRequestFrame(
		rem.TeamLoaderProtocolID,
		3,
		(&rem.LoadTeamChainArg{
			Team:  proto.FQTeam{Team: teamID, Host: hostID},
			Tok:   rem.NewTokenVariantWithLocalparentteam(parentToken),
			Start: proto.ChainEldestSeqno,
		}).Export(),
	)
	if err != nil {
		t.Fatalf("encode local-parent team-chain request: %v", err)
	}
	checkRPCFixture(t, "team-load-local-parent-request.frame", parentLoadFrame)
	request := rem.TeamVOBearerTokenReq{
		Team: proto.FQTeamIDOrName{
			Host:     hostID,
			IdOrName: proto.NewTeamIDOrNameWithTrue(teamID.EntityID()),
		},
		Member:  proto.FQParty{Party: uid.ToPartyID(), Host: hostID},
		SrcRole: proto.OwnerRole,
		Gen:     proto.FirstGeneration + 1,
	}
	frame, err = rpcRequestFrame(
		rem.TeamLoaderProtocolID,
		0,
		(&rem.GetTeamVOBearerTokenChallengeArg{Req: request}).Export(),
	)
	if err != nil {
		t.Fatalf("encode team-view challenge request: %v", err)
	}
	path = filepath.Join(checkedUserFixtureDirectory, "team-view-challenge-request.frame")
	if os.Getenv("FOKS_UPDATE_RPC_FIXTURES") == "1" {
		if err := os.WriteFile(path, frame, 0o644); err != nil {
			t.Fatalf("update team-view challenge request fixture: %v", err)
		}
	}
	want, err = os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(frame, want) {
		t.Fatal("checked team-view challenge request does not use the generated v0.1.9 method argument")
	}
}

func checkRPCFixture(t *testing.T, name string, got []byte) {
	t.Helper()
	path := filepath.Join(checkedUserFixtureDirectory, name)
	if os.Getenv("FOKS_UPDATE_RPC_FIXTURES") == "1" {
		if err := os.WriteFile(path, got, 0o644); err != nil {
			t.Fatalf("update %s: %v", name, err)
		}
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want) {
		t.Fatalf("%s differs from the generated v0.1.9 encoding", name)
	}
}
