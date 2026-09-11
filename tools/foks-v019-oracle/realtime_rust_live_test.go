package main

import (
	"bytes"
	"context"
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"github.com/foks-proj/go-foks/lib/core"
	p "github.com/foks-proj/go-foks/proto/lib"
	r "github.com/foks-proj/go-foks/proto/rem"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// The Rust testkit supplies a temporary account, named team, and pinned CA.
func TestGoRealtimeAgainstRustServer(t *testing.T) {
	dir := os.Getenv("FOKS_RT_LIVE_DIR")
	if dir == "" {
		t.Skip("run via the Rust realtime conformance scenario")
	}
	read := func(name string) []byte {
		b, e := os.ReadFile(filepath.Join(dir, name))
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
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	rpcClient, closeClient := liveRPCClient(t, ctx, os.Getenv("FOKS_RT_LIVE_ADDRESS"), liveRootPool(t, filepath.Join(dir, "ca.der")), cert)
	defer closeClient()
	cli := core.NewRealTimeClient(rpcClient, nil)
	var create r.RtNewChannelArg
	if e = core.DecodeFromBytes(&create, read("create.snowp")); e != nil {
		t.Fatal(e)
	}
	var nc p.RTMsgNoncer
	if e = core.DecodeFromBytes(&nc, read("noncer.snowp")); e != nil {
		t.Fatal(e)
	}
	if e = cli.RtSelectVHost(ctx, nc.Team.Host); e != nil {
		t.Fatal(e)
	}
	if e = cli.RtNewChannel(ctx, create); e != nil {
		t.Fatal(e)
	}
	channels, e := cli.RtListAllChannelsForTeam(ctx, r.RtListAllChannelsForTeamArg{Team: create.Md.ParentTeam, AppID: p.RTAppID_Chat})
	if e != nil {
		t.Fatal(e)
	}
	if len(channels.Lst) != 2 {
		t.Fatalf("expected two channels: %+v", channels)
	}
	derive := func(seed p.SecretSeed32, obj core.CryptoPayloader) p.SecretSeed32 {
		k, e := core.GenericDeriveKey32(seed, obj)
		if e != nil {
			t.Fatal(e)
		}
		return *k
	}
	var seed p.SecretSeed32
	for i := range seed {
		seed[i] = 0x52
	}
	kd := p.NewKeyDerivationDefault(p.KeyDerivationType_AppKey)
	app := derive(seed, &kd)
	ad := p.NewAppKeyDerivationWithEnum(p.AppKeyEnum_Realtime)
	rt := derive(app, &ad)
	d := p.RTKeyDerivation{App: p.RTAppID_Chat, Var: p.NewRTKeyVarDefault(p.RTKeyType_Data)}
	data := p.SecretBoxKey(derive(rt, &d))
	hash, e := core.PrefixedHash(&nc)
	if e != nil {
		t.Fatal(e)
	}
	nonce := hash.DomainSeparatedNaclNonce()
	body := p.NewRTMsgBodyWithBasic(p.RTMsgPlaintextBasic([]byte("hello from Go")))
	ct, e := core.SealIntoSecretBoxWithDomainSeparatedNonceAndPadding(&body, nonce, &data, 0)
	if e != nil {
		t.Fatal(e)
	}
	send := r.RTSendArg{Md: nc.Md, Chid: nc.Chid.Short(), Mw: p.NewRTMsgWrapperWithEncrypted(p.RTMsgBox{Ctext: p.NewRTMsgCiphertextWithNacl(ct), Rg: p.RoleAndGen{Role: p.DefaultRole, Gen: 1}})}
	receipt, e := cli.RtSend(ctx, send)
	if e != nil {
		t.Fatal(e)
	}
	replay, e := cli.RtSend(ctx, send)
	if e != nil || receipt != replay {
		t.Fatalf("replay mismatch: %+v %+v %v", receipt, replay, e)
	}
	inboxVersion, e := cli.RtGetInboxVersion(ctx, r.RTInboxKey{AppID: p.RTAppID_Chat})
	if e != nil || inboxVersion < 3 {
		t.Fatalf("inbox version: %d %v", inboxVersion, e)
	}
	delta, e := cli.RtGetChangedThreads(ctx, r.RTGetChangedThreadsArg{AppID: p.RTAppID_Chat, Max: 1})
	if e != nil || len(delta.Channels) != 1 || delta.InboxVersion != inboxVersion {
		t.Fatalf("inbox delta: %+v %v", delta, e)
	}
	if e = cli.RtReadThrough(ctx, r.RTReadThroughArg{ChannelID: nc.Chid, Seq: receipt.Seq}); e != nil {
		t.Fatal(e)
	}
	poll, e := cli.RtPollInbox(ctx, r.RTPollInboxArg{AppID: p.RTAppID_Chat, Since: 0, Timeout: 1})
	if e != nil || !poll.Bumped || poll.InboxVersion != inboxVersion {
		t.Fatalf("stale poll: %+v %v", poll, e)
	}
	poll, e = cli.RtPollInbox(ctx, r.RTPollInboxArg{AppID: p.RTAppID_Chat, Since: inboxVersion, Timeout: 1})
	if e != nil || poll.Bumped || poll.InboxVersion != inboxVersion {
		t.Fatalf("timeout poll: %+v %v", poll, e)
	}
	recent, e := cli.RtGetThreadRecents(ctx, r.RtGetThreadRecentsArg{Ch: nc.Chid, Lim: 100})
	if e != nil {
		t.Fatal(e)
	}
	if len(recent.Lst) != 1 {
		t.Fatalf("recents: %+v", recent)
	}
	page, e := cli.RtGetThread(ctx, r.RTThreadQuery{ChannelID: nc.Chid, Bookends: []r.RTThreadRangeBookends{{Start: 1, End: 1}}, Seqs: []p.RTMsgSeq{1}})
	if e != nil {
		t.Fatal(e)
	}
	if len(page.RangeMsgs) != 1 || len(page.SeqMsgs) != 1 {
		t.Fatalf("thread: %+v", page)
	}
	stored := page.SeqMsgs[0]
	box := stored.Mw.Encrypted()
	storedCT := box.Ctext.Nacl()
	var opened p.RTMsgBody
	if e = core.OpenSecretBoxWithDomainSeparatedNonceInto(&opened, storedCT, nonce, &data); e != nil {
		t.Fatal(e)
	}
	if !bytes.Equal(opened.Basic(), []byte("hello from Go")) {
		t.Fatal("decrypted body differs")
	}
}
