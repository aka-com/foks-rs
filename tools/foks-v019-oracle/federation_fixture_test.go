package main

import (
	"encoding/hex"
	"os"
	"testing"

	"github.com/foks-proj/go-foks/lib/core"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
)

func TestCheckedFederationRPCFixtures(t *testing.T) {
	var remoteHost proto.HostID
	remoteHost[0] = byte(proto.EntityType_Host)
	for i := 1; i < len(remoteHost); i++ {
		remoteHost[i] = 0x22
	}
	var viewee, viewer proto.UID
	viewee[0] = byte(proto.EntityType_User)
	viewer[0] = byte(proto.EntityType_User)
	for i := 1; i < len(viewee); i++ {
		viewee[i] = 0x33
		viewer[i] = 0x44
	}
	var token proto.PermissionToken
	for i := range token {
		token[i] = byte(0x50 + i)
	}
	var team proto.TeamID
	team[0] = byte(proto.EntityType_NamedTeam)
	for i := 1; i < len(team); i++ {
		team[i] = 0x66
	}
	var viewToken rem.TeamVOBearerToken
	for i := range viewToken {
		viewToken[i] = byte(0x70 + i)
	}
	var nonce proto.NaclNonce
	for i := range nonce {
		nonce[i] = byte(0x80 + i)
	}
	secretBox := proto.NewSecretBoxWithNacl(proto.NaclSecretBox{
		Nonce: nonce, Ciphertext: proto.NaclCiphertext{0x90, 0x91, 0x92, 0x93},
	})
	inner := proto.TeamRemoteMemberViewTokenInner{
		Member: proto.FQParty{Party: viewer.ToPartyID(), Host: remoteHost},
		PtkGen: proto.FirstGeneration, SecretBox: secretBox, PtkRole: proto.DefaultRole,
	}
	boxedPayload := proto.TeamRemoteMemberViewTokenBoxPayload{
		Tok: token, Party: inner.Member, Tm: proto.Time(123456789),
	}
	payload := rem.GrantRemoteViewPermissionPayload{
		Viewee: viewee.ToPartyID(),
		Viewer: proto.FQParty{Party: viewer.ToPartyID(), Host: remoteHost},
		Tm:     proto.Time(123456789),
	}
	var signature proto.Ed25519Signature
	for i := range signature {
		signature[i] = byte(i)
	}
	objects := []struct {
		name string
		make func() ([]byte, error)
		want string
	}{
		{
			name: "beacon_lookup",
			make: func() ([]byte, error) {
				return rpcRequestFrame(rem.BeaconProtocolID, 2,
					(&rem.BeaconLookupArg{HostID: remoteHost}).Export())
			},
			want: "48950500cebe314f3c0282a44461746191c421022222222222222222222222222222222222222222222222222222222222222222a648656164657282a15601a2663181a45665727301",
		},
		{
			name: "remote_user_chain",
			make: func() ([]byte, error) {
				arg := rem.LoadUserChainArg{
					Uid:   viewee,
					Start: proto.Seqno(1),
					Auth:  rem.NewLoadUserChainAuthWithToken(token),
				}
				return rpcRequestFrame(rem.RegProtocolID, 11,
					(&rem.RegLoadUserChainArg{A: arg}).Export())
			},
			want: "63950500cef7ab85f30b82a4446174619194c42101333333333333333333333333333333333333333333333333333333333333333301c0920181a131c411505152535455565758595a5b5c5d5e5f60a648656164657282a15601a2663181a45665727301",
		},
		{
			name: "grant_remote_user_view",
			make: func() ([]byte, error) {
				return rpcRequestFrame(rem.UserProtocolID, 18,
					(&rem.GrantRemoteViewPermissionForUserArg{P: payload}).Export())
			},
			want: "cc95950500ce823f08991282a4446174619193c42101333333333333333333333333333333333333333333333333333333333333333392c421014444444444444444444444444444444444444444444444444444444444444444c421022222222222222222222222222222222222222222222222222222222222222222ce075bcd15a648656164657282a15601a2663181a45665727301",
		},
		{
			name: "grant_remote_team_view",
			make: func() ([]byte, error) {
				arg := rem.GrantRemoteViewPermissionForTeamArg{
					P: payload,
					Sig: rem.SharedKeySig{
						Sig:  proto.NewSignatureWithEddsa(signature),
						Gen:  proto.FirstGeneration,
						Role: proto.OwnerRole,
					},
				}
				return rpcRequestFrame(rem.TeamMemberProtocolID, 1, arg.Export())
			},
			want: "cce1950500cebda3b9d30182a4446174619293c42101333333333333333333333333333333333333333333333333333333333333333392c421014444444444444444444444444444444444444444444444444444444444444444c421022222222222222222222222222222222222222222222222222222222222222222ce075bcd1593920081a130c440000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f01920380a648656164657282a15601a2663181a45665727301",
		},
		{
			name: "remote_team_chain",
			make: func() ([]byte, error) {
				arg := rem.LoadTeamChainArg{
					Team: proto.FQTeam{Team: team, Host: remoteHost},
					Tok: rem.NewTokenVariantWithPermission(token), Start: proto.ChainEldestSeqno,
				}
				return rpcRequestFrame(rem.TeamLoaderProtocolID, 3, arg.Export())
			},
			want: "cc89950500cef91285790382a4446174619792c421036666666666666666666666666666666666666666666666666666666666666666c421022222222222222222222222222222222222222222222222222222222222222222920281a131c411505152535455565758595a5b5c5d5e5f6001c0c0c2c2a648656164657282a15601a2663181a45665727301",
		},
		{
			name: "remote_member_payload",
			make: func() ([]byte, error) { return core.EncodeToBytes(&boxedPayload) },
			want: "93c411505152535455565758595a5b5c5d5e5f6092c421014444444444444444444444444444444444444444444444444444444444444444c421022222222222222222222222222222222222222222222222222222222222222222ce075bcd15",
		},
		{
			name: "remote_token_set",
			make: func() ([]byte, error) {
				set := rem.TeamRemoteViewTokenSet{Tokens: []proto.TeamRemoteMemberViewTokenInner{inner}}
				return core.EncodeToBytes(&set)
			},
			want: "91919492c421014444444444444444444444444444444444444444444444444444444444444444c42102222222222222222222222222222222222222222222222222222222222222222201920081a13092c410808182838485868788898a8b8c8d8e8fc40490919293920181a13000",
		},
		{
			name: "load_remote_token_boxes",
			make: func() ([]byte, error) {
				arg := rem.LoadTeamRemoteViewTokensArg{
					Team: proto.FQTeam{Team: team, Host: remoteHost}, Tok: viewToken,
					Members: []proto.FQParty{inner.Member},
				}
				return rpcRequestFrame(rem.TeamLoaderProtocolID, 6, arg.Export())
			},
			want: "ccc6950500cef91285790682a4446174619392c421036666666666666666666666666666666666666666666666666666666666666666c421022222222222222222222222222222222222222222222222222222222222222222c410707172737475767778797a7b7c7d7e7f9192c421014444444444444444444444444444444444444444444444444444444444444444c421022222222222222222222222222222222222222222222222222222222222222222a648656164657282a15601a2663181a45665727301",
		},
	}
	for _, object := range objects {
		got, err := object.make()
		if err != nil {
			t.Fatalf("%s: %v", object.name, err)
		}
		encoded := hex.EncodeToString(got)
		if os.Getenv("FOKS_PRINT_FEDERATION_FIXTURES") == "1" {
			t.Logf("%s=%s", object.name, encoded)
			continue
		}
		if encoded != object.want {
			t.Fatalf("%s changed:\n%s", object.name, encoded)
		}
	}
}
