package main

import (
	"bytes"
	cryptorand "crypto/rand"
	"encoding/json"
	"github.com/foks-proj/go-foks/lib/core"
	p "github.com/foks-proj/go-foks/proto/lib"
	r "github.com/foks-proj/go-foks/proto/rem"
	"github.com/foks-proj/go-snowpack-rpc/rpc"
	"os"
	"path/filepath"
	"testing"
)

func TestAccountFixtures(t *testing.T) {
	root := os.Getenv("FOKS_ACCOUNT_FIXTURE_OUT")
	generate := root != ""
	if !generate {
		root = "../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/account"
	}
	if generate {
		if e := os.MkdirAll(root, 0755); e != nil {
			t.Fatal(e)
		}
	}
	put := func(name string, b []byte) {
		t.Helper()
		path := filepath.Join(root, name)
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
				t.Fatalf("changed fixture %s", name)
			}
		}
	}
	emit := func(name string, v core.Codecable) {
		t.Helper()
		b, e := core.EncodeToBytes(v)
		if e != nil {
			t.Fatal(e)
		}
		put(name, b)
	}
	var host p.HostID
	host[0] = 2
	host[1] = 9
	for _, name := range []string{"zero", "incremental", "high"} {
		var seed p.BotTokenSeed
		for i := range seed {
			if name == "incremental" {
				seed[i] = byte(i)
			}
			if name == "high" {
				seed[i] = 255
			}
		}
		var token core.BotToken
		if e := token.FromSeed(seed); e != nil {
			t.Fatal(e)
		}
		txt, e := token.Export()
		if e != nil {
			t.Fatal(e)
		}
		var derived p.SecretSeed32
		if e := token.SecretSeed32(&derived); e != nil {
			t.Fatal(e)
		}
		key, e := token.KeySuite(p.OwnerRole, host)
		if e != nil {
			t.Fatal(e)
		}
		id, e := key.EntityID()
		if e != nil {
			t.Fatal(e)
		}
		hepk, e := key.ExportHEPK()
		if e != nil {
			t.Fatal(e)
		}
		prefix := "bot-" + name + "."
		put(prefix+"seed", seed[:])
		put(prefix+"txt", []byte(txt))
		put(prefix+"derived", derived[:])
		put(prefix+"id", id[:])
		emit(prefix+"hepk", hepk)
		host2 := host
		host2[1]++
		key2, e := token.KeySuite(p.OwnerRole, host2)
		if e != nil {
			t.Fatal(e)
		}
		id2, e := key2.EntityID()
		if e != nil || !bytes.Equal(id[:], id2[:]) {
			t.Fatal("token keys unexpectedly host salted", e)
		}
	}
	norms := map[string]string{}
	for _, name := range []string{"alice", "ALICE", "Álice", "alice smith", "Ａlice", "ab", "a.b"} {
		n, e := core.NormalizeName(p.NameUtf8(name))
		if e != nil {
			norms[name] = ""
		} else {
			norms[name] = string(n)
		}
	}
	b, e := json.Marshal(norms)
	if e != nil {
		t.Fatal(e)
	}
	put("normalization.json", b)
	var seed p.SecretSeed32
	seed[0] = 3
	key, e := core.NewPrivateSuite25519(p.EntityType_Device, p.OwnerRole, seed, host)
	if e != nil {
		t.Fatal(e)
	}
	var uid p.UID
	uid[0] = 1
	uid[1] = 7
	deterministicFixtureMu.Lock()
	defer deterministicFixtureMu.Unlock()
	old := cryptorand.Reader
	cryptorand.Reader = &deterministicFixtureReader{}
	defer func() { cryptorand.Reader = old }()
	made, e := core.MakeChangeUsernameLink(uid, host, key, r.NameCommitment{Name: "alice_new", Seq: 1}, 2, p.LinkHash{4}, p.TreeRoot{Epno: 3})
	if e != nil {
		t.Fatal(e)
	}
	link, e := retimeUserGroupLink(made.Link, []core.Signer{key})
	if e != nil {
		t.Fatal(e)
	}
	arg := r.ChangeUsernameArg{UsernameUtf8: "Alice_New", Full: &r.ChangedUsernameFullUpdateArg{Link: *link, UsernameCommitmentKey: *made.UsernameCommitmentKey, Rur: r.ReserveNameRes{Tok: p.ReservationToken{35, 4}, Seq: 1, Etime: 1700000030000}, NextTreeLocation: *made.NextTreeLocation}}
	emit("rename-full.snowp", &arg)
	frame, e := rpcRequestFrameAt(r.UserProtocolID, 11, arg.Export(), 7)
	if e != nil {
		t.Fatal(e)
	}
	put("rename-full.frame", frame)
	arg.Full = nil
	arg.UsernameUtf8 = "ALICE"
	emit("rename-display.snowp", &arg)
	frame, e = rpcRequestFrameAt(r.UserProtocolID, 11, arg.Export(), 7)
	if e != nil {
		t.Fatal(e)
	}
	put("rename-display.frame", frame)
	for _, c := range []struct {
		name     string
		protocol rpc.ProtocolUniqueID
		method   rpc.Position
		arg      interface{}
	}{
		{"host-id", r.RegProtocolID, 3, (&r.GetHostIDArg{}).Export()},
		{"vhost-mgmt", r.RegProtocolID, 22, (&r.GetVHostMgmtHostArg{}).Export()},
		{"web-admin", r.UserProtocolID, 21, (&r.NewWebAdminPanelURLArg{}).Export()},
		{"check-url", r.UserProtocolID, 22, (&r.CheckURLArg{Url: "https://admin.example/a?tok=x"}).Export()},
		{"reserve", r.UserProtocolID, 12, (&r.ReserveUsernameForChangeArg{Un: "alice_new"}).Export()},
		{"location", r.UserProtocolID, 13, (&r.GetTreeLocationArg{Seqno: 2}).Export()},
	} {
		frame, e := rpcRequestFrameAt(c.protocol, c.method, c.arg, 7)
		if e != nil {
			t.Fatal(e)
		}
		put(c.name+".frame", frame)
	}

}
