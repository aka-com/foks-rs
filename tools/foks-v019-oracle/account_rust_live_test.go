package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"github.com/foks-proj/go-foks/lib/core"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
	"testing"
)

func liveRename(t *testing.T, ctx context.Context, client *rem.UserClient, merkle *rem.MerkleQueryClient, user *goLiveUser) {
	t.Helper()
	reservation, err := client.ReserveUsernameForChange(ctx, "gorenamed")
	if err != nil {
		t.Fatal(err)
	}
	made, err := core.MakeChangeUsernameLink(user.uid, user.host, user.device, rem.NameCommitment{Name: "gorenamed", Seq: reservation.Seq}, user.nextSeqno, user.prev, liveCurrentRoot(t, ctx, merkle, user.host))
	if err != nil {
		t.Fatal(err)
	}
	err = client.ChangeUsername(ctx, rem.ChangeUsernameArg{UsernameUtf8: "GoRenamed", Full: &rem.ChangedUsernameFullUpdateArg{Link: *made.Link, UsernameCommitmentKey: *made.UsernameCommitmentKey, Rur: reservation, NextTreeLocation: *made.NextTreeLocation}})
	if err != nil {
		t.Fatal(err)
	}
	resolved, err := client.ResolveUsername(ctx, rem.ResolveUsernameArg{N: "gorenamed", Auth: rem.NewLoadUserChainAuthWithOpenvhost()})
	if err != nil || !resolved.Eq(user.uid) {
		t.Fatalf("renamed identity: %v", err)
	}
	if _, err = client.ResolveUsername(ctx, rem.ResolveUsernameArg{N: "gocompat", Auth: rem.NewLoadUserChainAuthWithOpenvhost()}); err == nil {
		t.Fatal("old name still resolves")
	}
	loaded, err := client.LoadUserChain(ctx, rem.LoadUserChainArg{Uid: user.uid, Start: proto.ChainEldestSeqno, Auth: rem.NewLoadUserChainAuthWithAslocaluser()})
	if err != nil || len(loaded.Links) != int(user.nextSeqno) {
		t.Fatalf("renamed history: %v", err)
	}
	hash, err := core.LinkHash(made.Link)
	if err != nil {
		t.Fatal(err)
	}
	user.prev = *hash
	user.nextSeqno++
	if err = client.ChangeUsername(ctx, rem.ChangeUsernameArg{UsernameUtf8: "GORENAMED"}); err != nil {
		t.Fatal(err)
	}
	if err = client.ChangeUsername(ctx, rem.ChangeUsernameArg{UsernameUtf8: "GORENAMED"}); err == nil {
		t.Fatal("identical display should return NoChange")
	}
}

func liveBot(t *testing.T, ctx context.Context, owner *rem.UserClient, reg *rem.RegClient, merkle *rem.MerkleQueryClient, user *goLiveUser, address string, roots *x509.CertPool) {
	for index, role := range []proto.Role{proto.OwnerRole, proto.DefaultRole} {
		var introduced core.SharedPrivateSuiter
		if index == 1 {
			var seed proto.SecretSeed32
			for i := range seed {
				seed[i] = byte(31 + i)
			}
			var err error
			introduced, err = core.NewSharedPrivateSuite25519(proto.EntityType_PUKVerify, role, seed, proto.FirstGeneration, user.host)
			if err != nil {
				t.Fatal(err)
			}
		}
		token, err := core.NewBotToken()
		if err != nil {
			t.Fatal(err)
		}
		key, err := token.KeySuite(role, user.host)
		if err != nil {
			t.Fatal(err)
		}
		label, err := token.DeviceLabelAndName()
		if err != nil {
			t.Fatal(err)
		}
		liveProvisionKey(t, ctx, owner, reg, merkle, user, key, label.Label, label.Name, role, introduced)
		id, err := key.EntityID()
		if err != nil {
			t.Fatal(err)
		}
		challenge, err := reg.GetUIDLookupChallege(ctx, id)
		if err != nil {
			t.Fatal(err)
		}
		if !challenge.Payload.EntityID.Eq(id) || !challenge.Payload.HostID.Eq(user.host) || !challenge.Payload.Time.IsNowish() {
			t.Fatal("bot challenge binding")
		}
		sig, err := key.Sign(&challenge.Payload)
		if err != nil {
			t.Fatal(err)
		}
		lookup, err := reg.LookupUIDByDevice(ctx, rem.LookupUIDByDeviceArg{EntityID: id, Challenge: challenge, Signature: *sig})
		if err != nil {
			t.Fatal(err)
		}
		if !lookup.Fqu.Uid.Eq(user.uid) || !lookup.Fqu.HostID.Eq(user.host) {
			t.Fatal("bot lookup binding")
		}
		certs, err := reg.GetClientCertChain(ctx, rem.GetClientCertChainArg{Uid: user.uid, Key: id})
		if err != nil {
			t.Fatal(err)
		}
		private, err := key.PrivateKeyForCert()
		if err != nil {
			t.Fatal(err)
		}
		conn, close := liveRPCClient(t, ctx, address, roots, &tls.Certificate{Certificate: certs, PrivateKey: private})
		defer close()
		bot := core.NewUserClient(conn, nil)
		uid, err := bot.Ping(ctx)
		if err != nil || !uid.Eq(user.uid) {
			t.Fatalf("bot authentication: %v", err)
		}
		kv := core.NewKVStoreClient(conn, nil)
		if _, err = kv.KvUsage(ctx, rem.KVAuth{}); err != nil {
			t.Fatal(err)
		}
		if _, err = bot.GetPUKForRole(ctx, rem.GetPUKForRoleArg{Role: role, TargetPublicKeyId: id}); err != nil {
			t.Fatal(err)
		}
		if index == 1 {
			if _, err = bot.GetPUKForRole(ctx, rem.GetPUKForRoleArg{Role: proto.OwnerRole, TargetPublicKeyId: id}); err == nil {
				t.Fatal("member bot opened owner PUK")
			}
		}
	}
	t.Log("official Go bot enrollment, challenge lookup, authentication and KV succeeded")
}
