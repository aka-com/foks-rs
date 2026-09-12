package main

import (
	"bytes"
	"context"
	"crypto/tls"
	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/sso"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
	"io"
	"net/http"
	"os"
	"testing"
	"time"
)

func liveOidcFlow(t *testing.T, ctx context.Context, reg *rem.RegClient, merkle *rem.MerkleQueryClient, host proto.HostID, uid proto.UID, device core.PrivateSuiter, login bool) (rem.RegSSOArgs, rem.ReserveNameRes) {
	t.Helper()
	session, err := sso.PrepOAuth2Session(proto.FQUser{Uid: uid, HostID: host}, liveCurrentRoot(t, ctx, merkle, host))
	if err != nil {
		t.Fatal(err)
	}
	var uidp *proto.UID
	if login {
		uidp = &uid
	}
	url, err := reg.InitOAuth2Session(ctx, rem.InitOAuth2SessionArg{Id: session.Id, PkceVerifier: session.Verifier, Nonce: session.Nonce, Uid: uidp})
	if err != nil {
		t.Fatal(err)
	}
	browser := http.Client{Timeout: 15 * time.Second}
	response, err := browser.Get(url.String())
	if err != nil {
		t.Fatal(err)
	}
	_, _ = io.Copy(io.Discard, io.LimitReader(response.Body, 1<<20))
	response.Body.Close()
	if response.StatusCode != 200 {
		t.Fatalf("OIDC browser HTTP status %d", response.StatusCode)
	}
	poll, err := reg.PollOAuth2SessionCompletion(ctx, rem.PollOAuth2SessionCompletionArg{Id: session.Id, Wait: 1000, ForLogin: login})
	if err != nil {
		t.Fatal(err)
	}
	duplicate, err := reg.PollOAuth2SessionCompletion(ctx, rem.PollOAuth2SessionCompletionArg{Id: session.Id, Wait: 1000, ForLogin: login})
	if err != nil {
		t.Fatal(err)
	}
	first, _ := core.EncodeToBytes(&poll)
	second, _ := core.EncodeToBytes(&duplicate)
	if !bytes.Equal(first, second) {
		t.Fatal("duplicate poll changed tokens or reservation")
	}
	payload := proto.OAuth2IDTokenBindingPayload{IdToken: poll.Toks.IdToken, Binding: session.Binding}
	sig, inner, err := core.Sign2(device, &payload)
	if err != nil {
		t.Fatal(err)
	}
	key, err := device.EntityID()
	if err != nil {
		t.Fatal(err)
	}
	return rem.NewRegSSOArgsWithOauth2(rem.RegSSOArgsOAuth2{Id: session.Id, Sig: proto.OAuth2IDTokenBinding{Inner: *inner, Sig: *sig, Key: key}}), poll.Res
}
func TestGoSSOAgainstRustServer(t *testing.T) {
	probeAddress := os.Getenv("FOKS_GO_RUST_PROBE")
	caPath := os.Getenv("FOKS_GO_RUST_CA_DER")
	if probeAddress == "" || caPath == "" {
		t.Skip("run through the Rust OIDC live gate")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	probeRPC, closeProbe := liveRPCClient(t, ctx, probeAddress, liveRootPool(t, caPath), nil)
	defer closeProbe()
	probeClient := core.NewProbeClient(probeRPC, nil)
	probe, err := probeClient.Probe(ctx, rem.ProbeArg{Hostname: proto.Hostname("localhost")})
	if err != nil {
		t.Fatal(err)
	}
	chain, err := core.PlayChain(proto.TCPAddr(probeAddress), probe.Hostchain, nil)
	if err != nil {
		t.Fatal(err)
	}
	zone, err := core.CheckZoneSig(*chain, probe)
	if err != nil {
		t.Fatal(err)
	}
	roots, err := chain.RootCACertPool()
	if err != nil {
		t.Fatal(err)
	}
	public, closePublic := liveRPCClient(t, ctx, string(zone.Services.Reg), roots, nil)
	defer closePublic()
	reg := core.NewRegClient(public, nil)
	merkle := core.NewMerkleQueryClient(public, nil)
	config, err := reg.GetServerConfig(ctx)
	if err != nil || config.Sso == nil || config.Sso.Oauth2 == nil {
		t.Fatalf("missing OIDC config: %v", err)
	}
	if config.Sso.Oauth2.ClientSecret != "" {
		t.Fatal("server leaked client secret")
	}
	user := liveSignup(t, ctx, &reg, &merkle, chain.HostID())
	key, err := user.device.EntityID()
	if err != nil {
		t.Fatal(err)
	}
	cert, err := reg.GetClientCertChain(ctx, rem.GetClientCertChainArg{Uid: user.uid, Key: key})
	if err != nil {
		t.Fatal(err)
	}
	private, err := user.device.PrivateKeyForCert()
	if err != nil {
		t.Fatal(err)
	}
	auth, closeAuth := liveRPCClient(t, ctx, string(zone.Services.User), roots, &tls.Certificate{Certificate: cert, PrivateKey: private})
	defer closeAuth()
	client := core.NewUserClient(auth, nil)
	if _, err := client.Ping(ctx); err != nil {
		t.Fatal(err)
	}
	args, _ := liveOidcFlow(t, ctx, &reg, &merkle, user.host, user.uid, user.device, true)
	if err := reg.SsoLogin(ctx, rem.SsoLoginArg{Uid: user.uid, Args: args}); err != nil {
		t.Fatal(err)
	}
	if err := reg.SsoLogin(ctx, rem.SsoLoginArg{Uid: user.uid, Args: args}); err != nil {
		t.Fatalf("identical login replay: %v", err)
	}
	if _, err := client.Ping(ctx); err != nil {
		t.Fatal(err)
	}
}
