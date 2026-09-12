package main

import (
	"bytes"
	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/team"
	p "github.com/foks-proj/go-foks/proto/lib"
	r "github.com/foks-proj/go-foks/proto/rem"
	"os"
	"path/filepath"
	"testing"
)

func TestInvitationCertificateFixtures(t *testing.T) {
	root := os.Getenv("FOKS_INVITATION_FIXTURE_OUT")
	generate := root != ""
	if !generate {
		root = "../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/invitations"
	}
	if generate {
		if err := os.MkdirAll(root, 0755); err != nil {
			t.Fatal(err)
		}
	}
	put := func(n string, b []byte) {
		t.Helper()
		path := filepath.Join(root, n)
		if generate {
			if e := os.WriteFile(path, b, 0644); e != nil {
				t.Fatal(e)
			}
		} else {
			old, e := os.ReadFile(path)
			if e != nil {
				t.Fatal(e)
			}
			if !bytes.Equal(old, b) {
				t.Fatalf("fixture drift %s", n)
			}
		}
	}
	var host p.HostID
	host[0] = 2
	host[1] = 44
	keys := make([]core.SharedPrivateSuiter, 2)
	for i := range keys {
		var seed p.SecretSeed32
		for j := range seed {
			seed[j] = byte(j + 11 + i*32)
		}
		k, e := core.NewSharedPrivateSuite25519(p.EntityType_PTKVerify, p.AdminRole, seed, p.Generation(i+1), host)
		if e != nil {
			t.Fatal(e)
		}
		keys[i] = k
	}
	id, e := keys[0].EntityID()
	if e != nil {
		t.Fatal(e)
	}
	id[0] = byte(p.EntityType_NamedTeam)
	tid, e := id.ToTeamID()
	if e != nil {
		t.Fatal(e)
	}

	emit := func(n string, v core.Codecable) {
		t.Helper()
		b, e := core.EncodeToBytes(v)
		if e != nil {
			t.Fatal(e)
		}
		put(n, b)
	}
	var uid p.UID
	uid[0] = 1
	uid[1] = 45
	var permission p.PermissionToken
	permission[0] = 55
	permission[1] = 46
	grant := r.GrantLocalViewPermissionPayload{Viewee: uid.ToPartyID(), Viewer: tid.ToPartyID(), Tm: 1700000000000, ViewerRole: &p.AdminRole}
	emit("local-grant.payload", &grant)
	var receipt p.TeamRSVPLocal
	receipt[0] = 57
	receipt[1] = 47
	inbox := r.TeamRawInbox{Rows: []r.TeamRawInboxRow{{Time: 1700000000000, State: r.JoinreqState_Pending, Row: r.NewTeamRawInboxRowVarWithLocal(r.TeamRawInboxRowLocal{Tok: receipt, Joiner: uid.ToPartyID(), SrcRole: p.OwnerRole, Perm: permission})}}}
	emit("local.inbox", &inbox)
	payload := r.TeamRemoteJoinReqPayload{Joiner: p.FQParty{Party: uid.ToPartyID(), Host: host}, Tok: permission, Tm: 1700000000000, SrcRole: p.OwnerRole}
	emit("remote.payload", &payload)
	remotePath := filepath.Join(root, "remote.request")
	var remote r.TeamRemoteJoinReq
	if generate {
		boxer, e := core.PublicizeToSPSBoxer(keys[0], p.FQParty{Party: tid.ToPartyID(), Host: host})
		if e != nil {
			t.Fatal(e)
		}
		box, e := keys[1].BoxFor(&payload, boxer, core.BoxOpts{IncludePublicKey: true})
		if e != nil {
			t.Fatal(e)
		}
		_, hepk, e := keys[0].ExportToSharedKey()
		if e != nil {
			t.Fatal(e)
		}
		fp, e := core.HEPK(hepk).Fingerprint()
		if e != nil {
			t.Fatal(e)
		}
		remote = r.TeamRemoteJoinReq{HepkFp: *fp, Box: *box, Vd: payload.Vd}
		emit("remote.request", &remote)
	} else {
		b, e := os.ReadFile(remotePath)
		if e != nil {
			t.Fatal(e)
		}
		if e = core.DecodeFromBytes(&remote, b); e != nil {
			t.Fatal(e)
		}
	}
	var opened r.TeamRemoteJoinReqPayload
	if _, e := keys[0].UnboxFor(&opened, remote.Box, nil); e != nil {
		t.Fatal(e)
	}
	equal, e := core.Eq(&opened, &payload)
	if e != nil || !equal {
		t.Fatalf("remote payload mismatch: %v", e)
	}
	for i, k := range keys {
		name := []string{"initial", "rotated"}[i]
		c, e := team.MakeTeamCert(p.FQTeam{Host: host, Team: tid}, keys[0], k, p.NameUtf8("Invitation fixture"))
		if e != nil {
			t.Fatal(e)
		}
		signed := c.V1()
		payload, e := signed.Payload.AllocAndDecode(core.DecoderFactory{})
		if e != nil {
			t.Fatal(e)
		}
		payload.Tm = 1700000000000
		bl, e := payload.EncodeTyped(core.EncoderFactory{})
		if e != nil {
			t.Fatal(e)
		}
		signed.Payload = *bl
		signers := []core.Signer{k}
		if i > 0 {
			signers = append(signers, keys[0])
		}
		if e = core.SignStacked(&signed, signers); e != nil {
			t.Fatal(e)
		}
		*c = r.NewTeamCertWithV1(signed)
		if _, e = team.OpenTeamCert(*c); e != nil {
			t.Fatal(e)
		}
		// The pinned client/libclient openCert uses the reverse verifier order.
		first, e := core.ImportEntityPublic(tid.EntityID())
		if e != nil {
			t.Fatal(e)
		}
		verifiers := []core.Verifier{first}
		if i > 0 {
			current, e := core.ImportEntityPublic(payload.Ptk.VerifyKey)
			if e != nil {
				t.Fatal(e)
			}
			verifiers = append(verifiers, current)
		}
		clientErr := core.VerifyStackedSignature(&signed, verifiers)
		if (i == 0) != (clientErr == nil) {
			t.Fatalf("unexpected pinned client stack result: %v", clientErr)
		}
		b, e := core.EncodeToBytes(c)
		if e != nil {
			t.Fatal(e)
		}
		put(name+".cert", b)
		var hash p.TeamCertHash
		if e = core.PrefixedHashInto(c, hash[:]); e != nil {
			t.Fatal(e)
		}
		invite := p.NewTeamInviteWithV1(p.TeamInviteV1{Hsh: hash, Host: host})
		b, e = core.EncodeToBytes(&invite)
		if e != nil {
			t.Fatal(e)
		}
		put(name+".invite", b)
		txt, e := team.ExportTeamInvite(invite)
		if e != nil {
			t.Fatal(e)
		}
		put(name+".txt", []byte(txt))
		if i == 0 {
			var tok r.TeamBearerToken
			tok[0] = 91
			frame, e := rpcRequestFrame(r.TeamGuestProtocolID, 0, (&r.LookupTeamCertByHashArg{I: invite}).Export())
			if e != nil {
				t.Fatal(e)
			}
			put("lookup.rpc", frame)
			frame, e = rpcRequestFrame(r.TeamAdminProtocolID, 6, (&r.PutTeamCertArg{Tok: tok, Cert: *c}).Export())
			if e != nil {
				t.Fatal(e)
			}
			put("put.rpc", frame)
			frame, e = rpcRequestFrame(r.TeamMemberProtocolID, 0, (&r.AcceptInviteLocalArg{I: invite, SrcRole: p.OwnerRole}).Export())
			if e != nil {
				t.Fatal(e)
			}
			put("accept-local.rpc", frame)
			frame, e = rpcRequestFrame(r.UserProtocolID, 10, (&r.GrantLocalViewPermissionForUserArg{P: grant}).Export())
			if e != nil {
				t.Fatal(e)
			}
			put("grant-user.rpc", frame)
		}

	}
}
