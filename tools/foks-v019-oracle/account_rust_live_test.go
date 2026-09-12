package main

import (
	"context"
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
	if err = client.ChangeUsername(ctx, rem.ChangeUsernameArg{UsernameUtf8: "GORENAMED"}); err != nil {
		t.Fatal(err)
	}
	if err = client.ChangeUsername(ctx, rem.ChangeUsernameArg{UsernameUtf8: "GORENAMED"}); err == nil {
		t.Fatal("identical display should return NoChange")
	}
}
