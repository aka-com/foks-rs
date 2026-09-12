package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/team"
	p "github.com/foks-proj/go-foks/proto/lib"
	r "github.com/foks-proj/go-foks/proto/rem"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// Uses the unmodified Go RPC clients and Go crypto to request membership, then
// prove membership after the Rust administrator approves the same inbox row.
func TestGoInvitationsAgainstRustServer(t *testing.T) {
	dir := os.Getenv("FOKS_INVITATION_LIVE_DIR")
	if dir == "" {
		t.Skip("run the Rust invitations_live gate with FOKS_GO_ORACLE_DIR")
	}
	read := func(n string) []byte {
		b, e := os.ReadFile(filepath.Join(dir, n))
		if e != nil {
			t.Fatal(e)
		}
		return b
	}
	key, e := x509.ParsePKCS8PrivateKey(read("key.der"))
	if e != nil {
		t.Fatal(e)
	}
	var chain [][]byte
	for i := 0; i < 16; i++ {
		b, e := os.ReadFile(filepath.Join(dir, fmt.Sprintf("certificate-%d.der", i)))
		if os.IsNotExist(e) {
			break
		}
		if e != nil {
			t.Fatal(e)
		}
		chain = append(chain, b)
	}
	cert := &tls.Certificate{Certificate: chain, PrivateKey: key}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	roots := liveRootPool(t, filepath.Join(dir, "ca.der"))
	public, closePublic := liveRPCClient(t, ctx, os.Getenv("FOKS_INVITATION_LIVE_PUBLIC"), roots, nil)
	defer closePublic()
	auth, closeAuth := liveRPCClient(t, ctx, os.Getenv("FOKS_INVITATION_LIVE_AUTH"), roots, cert)
	defer closeAuth()
	guest := r.TeamGuestClient{Cli: public, ErrorUnwrapper: core.StatusToError}
	member := r.TeamMemberClient{Cli: auth, ErrorUnwrapper: core.StatusToError}
	user := core.NewUserClient(auth, nil)
	merkle := core.NewMerkleQueryClient(public, nil)
	var invite p.TeamInvite
	if e = core.DecodeFromBytes(&invite, read("invite.snowp")); e != nil {
		t.Fatal(e)
	}
	found, e := guest.LookupTeamCertByHash(ctx, invite)
	if e != nil {
		t.Fatal(e)
	}
	opened, e := team.OpenTeamCert(found.Cert)
	if e != nil {
		t.Fatal(e)
	}
	var uid p.UID
	copy(uid[:], read("uid.raw"))
	var seed p.SecretSeed32
	copy(seed[:], read("seed.raw"))
	device, e := core.NewPrivateSuite25519(p.EntityType_Device, p.OwnerRole, seed, opened.Team.Host)
	if e != nil {
		t.Fatal(e)
	}
	if os.Getenv("FOKS_INVITATION_LIVE_PHASE") == "request" {
		_, e = user.GrantLocalViewPermissionForUser(ctx, r.GrantLocalViewPermissionPayload{Viewee: uid.ToPartyID(), Viewer: opened.Team.Team.ToPartyID(), Tm: p.Now(), ViewerRole: &p.AdminRole})
		if e != nil {
			t.Fatal(e)
		}
		root := liveCurrentRoot(t, ctx, &merkle, opened.Team.Host)
		link, e := core.MakeGenericLink(uid.EntityID(), opened.Team.Host, device, p.NewGenericLinkPayloadWithTeammembership(p.TeamMembershipLink{
			Team: opened.Team, SrcRole: p.OwnerRole, State: p.NewTeamMembershipDetailsDefault(p.TeamMembershipLinkState_Requested),
		}), p.ChainEldestSeqno, nil, root)
		if e != nil {
			t.Fatal(e)
		}
		arg := r.AcceptInviteLocalArg{I: invite, SrcRole: p.OwnerRole, TeamMembershipLink: &r.PostGenericLinkArg{Link: *link.Link, NextTreeLocation: *link.NextTreeLocation}}
		receipt, e := member.AcceptInviteLocal(ctx, arg)
		if e != nil {
			t.Fatal(e)
		}
		if receipt[0] != 57 {
			t.Fatal("wrong local RSVP type")
		}
		if _, e = member.AcceptInviteLocal(ctx, arg); e == nil {
			t.Fatal("duplicate pending invitation accepted")
		}
		t.Log("Go-generated Requested link and local view grant accepted by Rust")
		return
	}
	var pukSeed p.SecretSeed32
	copy(pukSeed[:], read("puk.raw"))
	puk, e := core.NewSharedPrivateSuite25519(p.EntityType_PUKVerify, p.OwnerRole, pukSeed, 1, opened.Team.Host)
	if e != nil {
		t.Fatal(e)
	}
	loader := r.TeamLoaderClient{Cli: auth, ErrorUnwrapper: core.StatusToError}
	challenge, e := loader.GetTeamVOBearerTokenChallenge(ctx, r.TeamVOBearerTokenReq{
		Team:   p.FQTeamIDOrName{Host: opened.Team.Host, IdOrName: p.NewTeamIDOrNameWithTrue(opened.Team.Team.EntityID())},
		Member: p.FQParty{Party: uid.ToPartyID(), Host: opened.Team.Host}, SrcRole: p.OwnerRole, Gen: 1,
	})
	if e != nil {
		t.Fatal(e)
	}
	sig, e := puk.Sign(&challenge)
	if e != nil {
		t.Fatal(e)
	}
	token, e := loader.ActivateTeamVOBearerToken(ctx, r.ActivateTeamVOBearerTokenArg{Ch: challenge, Sig: *sig})
	if e != nil {
		t.Fatal(e)
	}
	loaded, e := loader.LoadTeamChain(ctx, r.LoadTeamChainArg{Team: opened.Team, Tok: r.NewTokenVariantWithTeamvobearer(token.Tok), Start: 1})
	if e != nil {
		t.Fatal(e)
	}
	if len(loaded.Links) != 2 {
		t.Fatalf("membership chain links: %d", len(loaded.Links))
	}
	t.Log("Go PUK proof authorizes Rust membership and chain load after approval")
}
