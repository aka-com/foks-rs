package main

import (
	"bytes"
	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/sso"
	p "github.com/foks-proj/go-foks/proto/lib"
	r "github.com/foks-proj/go-foks/proto/rem"
	"os"
	"path/filepath"
	"testing"
)

// Fixture regeneration is explicit; normal runs compare immutable captured bytes.
func TestSSOFixtures(t *testing.T) {
	root := os.Getenv("FOKS_SSO_FIXTURE_OUT")
	generate := root != ""
	if !generate {
		root = filepath.Join("..", "..", "crates", "foks-snowpack", "tests", "fixtures", "foks-v0.1.9", "sso")
	}
	if generate {
		if err := os.MkdirAll(root, 0755); err != nil {
			t.Fatal(err)
		}
	}
	put := func(name string, data []byte) {
		t.Helper()
		path := filepath.Join(root, name)
		if generate {
			if e := os.WriteFile(path, data, 0644); e != nil {
				t.Fatal(e)
			}
		} else {
			old, e := os.ReadFile(path)
			if e != nil {
				t.Fatal(e)
			}
			if !bytes.Equal(old, data) {
				t.Fatalf("fixture changed: %s", name)
			}
		}
	}
	emit := func(name string, obj core.Codecable) {
		t.Helper()
		b, e := core.EncodeToBytes(obj)
		if e != nil {
			t.Fatal(e)
		}
		put(name+".snowp", b)
	}
	var host p.HostID
	host[0] = 2
	host[1] = 9
	var uid p.UID
	uid[0] = 1
	uid[1] = 7
	var sid p.OAuth2SessionID
	sid[0] = byte(p.ID16Type_OAuth2Session)
	sid[1] = 8
	var cfgid p.SSOConfigID
	cfgid[0] = byte(p.ID16Type_SSOConfig)
	cfgid[1] = 10
	binding := p.OAuth2Binding{Fqu: p.FQUser{Uid: uid, HostID: host}, Root: p.TreeRoot{Epno: 17}, Rand: p.OAuth2Random{5}}
	binding.Root.Hash[0] = 6
	emit("binding", &binding)
	nonce, e := sso.HashOAuth2Binding(&binding)
	if e != nil {
		t.Fatal(e)
	}
	put("nonce.txt", []byte(nonce))
	verifier := p.OAuth2PKCEVerifier(sso.Base64URLEncode(bytes.Repeat([]byte{3}, 32)))
	challenge, e := sso.HashPKCE(verifier)
	if e != nil {
		t.Fatal(e)
	}
	put("verifier.txt", []byte(verifier))
	put("challenge.txt", []byte(challenge))
	cfg := p.SSOConfig{Active: p.SSOProtocolType_Oauth2, Oauth2: &p.OAuth2Config{Id: cfgid, ConfigURI: "https://idp.example/.well-known/openid-configuration", ClientID: "foks", ClientSecret: "fixture-secret", RedirectURI: "https://host.example/oauth2/callback"}}
	emit("config", &cfg)
	cfg.Oauth2.ClientSecret = ""
	emit("public-config", &cfg)
	toks := p.OAuth2TokenSet{AccessToken: "opaque-access", IdToken: "fixture.id.token", Expires: 1700000000000, Username: "alice"}
	emit("tokens", &toks)
	payload := p.OAuth2IDTokenBindingPayload{IdToken: toks.IdToken, Binding: binding}
	emit("binding-payload", &payload)
	var seed p.SecretSeed32
	for i := range seed {
		seed[i] = byte(i)
	}
	put("seed.bin", seed[:])
	key, e := core.NewPrivateSuite25519(p.EntityType_Device, p.OwnerRole, seed, host)
	if e != nil {
		t.Fatal(e)
	}
	sig, inner, e := core.Sign2(key, &payload)
	if e != nil {
		t.Fatal(e)
	}
	pub, e := key.Publicize(&host)
	if e != nil {
		t.Fatal(e)
	}
	signed := p.OAuth2IDTokenBinding{Inner: *inner, Sig: *sig, Key: pub.GetEntityID()}
	emit("signed-binding", &signed)
	args := r.NewRegSSOArgsWithOauth2(r.RegSSOArgsOAuth2{Id: sid, Sig: signed})
	emit("reg-sso", &args)
	none := r.NewRegSSOArgsWithNone()
	emit("reg-none", &none)
	poll := r.PollOAuth2SessionCompletionArg{Id: sid, Wait: 30000, ForLogin: true}
	emit("poll", &poll)
	init := r.InitOAuth2SessionArg{Id: sid, PkceVerifier: verifier, Nonce: nonce, Uid: &uid}
	emit("init-login", &init)
	b, e := rpcRequestFrameAt(r.RegProtocolID, 18, init.Export(), 7)
	if e != nil {
		t.Fatal(e)
	}
	put("request-18.frame", b)
	init.Uid = nil
	emit("init-signup", &init)
	login := r.SsoLoginArg{Uid: uid, Args: args}
	emit("login", &login)
	b, e = rpcRequestFrameAt(r.RegProtocolID, 19, poll.Export(), 7)
	if e != nil {
		t.Fatal(e)
	}
	put("request-19.frame", b)
	b, e = rpcRequestFrameAt(r.RegProtocolID, 20, login.Export(), 7)
	if e != nil {
		t.Fatal(e)
	}
	put("request-20.frame", b)
	res := r.OAuth2PollRes{Toks: toks, Res: r.ReserveNameRes{Tok: p.ReservationToken{35, 4}, Seq: 1, Etime: 1700000030000}}
	emit("poll-result", &res)
	res.Res = r.ReserveNameRes{}
	emit("poll-login-result", &res)
}
