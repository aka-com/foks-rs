package main

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/foks-proj/go-foks/lib/core"
	p "github.com/foks-proj/go-foks/proto/lib"
	r "github.com/foks-proj/go-foks/proto/rem"
	"github.com/foks-proj/go-snowpack-rpc/rpc"
)

// Explicit output opt-in; ordinary tests compare deterministic pinned fixtures.
func TestRealtimeFixtures(t *testing.T) {
	root := os.Getenv("FOKS_RT_FIXTURE_OUT")
	generate := root != ""
	if !generate {
		root = filepath.Join("..", "..", "crates", "foks-snowpack", "tests", "fixtures", "foks-v0.1.9", "realtime")
	}
	if generate {
		if err := os.MkdirAll(root, 0755); err != nil {
			t.Fatal(err)
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
			if !bytes.Equal(b, old) {
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
	derive := func(seed p.SecretSeed32, obj core.CryptoPayloader) p.SecretSeed32 {
		t.Helper()
		k, e := core.GenericDeriveKey32(seed, obj)
		if e != nil {
			t.Fatal(e)
		}
		return *k
	}
	var seed p.SecretSeed32
	for i := range seed {
		seed[i] = byte(i)
	}
	put("seed.bin", seed[:])
	kd := p.NewKeyDerivationDefault(p.KeyDerivationType_AppKey)
	app := derive(seed, &kd)
	put("app-key.bin", app[:])
	ad := p.NewAppKeyDerivationWithEnum(p.AppKeyEnum_Realtime)
	rt := derive(app, &ad)
	put("rt-seed.bin", rt[:])
	keys := make(map[p.RTKeyType]p.SecretBoxKey)
	for _, typ := range []p.RTKeyType{p.RTKeyType_ChannelName, p.RTKeyType_ChannelDesc, p.RTKeyType_Data} {
		d := p.RTKeyDerivation{App: p.RTAppID_Chat, Var: p.NewRTKeyVarDefault(typ)}
		k := derive(rt, &d)
		keys[typ] = p.SecretBoxKey(k)
		put(fmt.Sprintf("key-%d.bin", typ), k[:])
		emit(fmt.Sprintf("derivation-%d", typ), &d)
	}
	var host p.HostID
	host[0] = 2
	host[1] = 9
	user := make(p.PartyID, 33)
	user[0] = 1
	user[1] = 7
	var team p.TeamID
	team[0] = 3
	team[1] = 8
	var chid p.RTChannelID
	chid[0] = 0x81
	chid[15] = 0x22
	var mid p.RTMsgID
	mid[0] = 0x33
	sender := p.FQParty{Party: user, Host: host}
	nc := p.RTMsgNoncer{Md: p.RTMsgMetadata{MsgID: mid, SendTime: 1700000000000, Typ: p.RTMsgType_Basic}, Sender: &sender, AppID: p.RTAppID_Chat, Team: p.FQParty{Party: p.PartyID(team[:]), Host: host}, Chid: chid}
	emit("noncer", &nc)
	hash, e := core.PrefixedHash(&nc)
	if e != nil {
		t.Fatal(e)
	}
	nonce := hash.DomainSeparatedNaclNonce()
	put("message-nonce.bin", nonce[:])
	body := p.NewRTMsgBodyWithBasic(p.RTMsgPlaintextBasic([]byte("hello")))
	emit("body", &body)
	dataKey := keys[p.RTKeyType_Data]
	ct, e := core.SealIntoSecretBoxWithDomainSeparatedNonceAndPadding(&body, nonce, &dataKey, 0)
	if e != nil {
		t.Fatal(e)
	}
	put("message-ciphertext.bin", ct)
	var opened p.RTMsgBody
	if e = core.OpenSecretBoxWithDomainSeparatedNonceInto(&opened, ct, nonce, &dataKey); e != nil {
		t.Fatal(e)
	}
	var partial p.NaclNonce
	for i := range partial {
		partial[i] = byte(i + 16)
	}
	put("text-nonce.bin", partial[:])
	name := p.NewRTChannelNamePlaintextWithUtf8v1(p.RTChannelName(""))
	desc := p.NewRTChannelDescPlaintextWithUtf8v1(p.RTChannelDesc("Hello team"))
	emit("name", &name)
	emit("description", &desc)
	makeBox := func(typ p.RTKeyType, obj core.CryptoPayloader) p.SecretBox {
		k := keys[typ]
		ct, e := core.SealIntoSecretBoxWithNonce(obj, &partial, &k)
		if e != nil {
			t.Fatal(e)
		}
		return p.NewSecretBoxWithNacl(p.NaclSecretBox{Nonce: partial, Ciphertext: ct})
	}
	nameBox := makeBox(p.RTKeyType_ChannelName, &name)
	descBox := makeBox(p.RTKeyType_ChannelDesc, &desc)
	emit("name-box", &nameBox)
	emit("description-box", &descBox)
	rg := p.RoleAndGen{Role: p.DefaultRole, Gen: 1}
	mw := p.NewRTMsgWrapperWithEncrypted(p.RTMsgBox{Ctext: p.NewRTMsgCiphertextWithNacl(ct), Rg: rg})
	emit("wrapper", &mw)
	md := r.RTChannelMetadata{Id: chid, ParentTeam: team, AppID: p.RTAppID_Chat, Seqno: 1, NameBox: p.RTBoxRG{Rg: p.RoleAndGen{Role: p.MinRTRole, Gen: 1}, Box: nameBox}, DescBox: &p.RTBoxRG{Rg: rg, Box: descBox}, Roles: p.RolePair{Read: p.DefaultRole, Write: p.DefaultRole}, Ctime: 1700000000000, Mtime: 1700000000001, UpdatedAt: 1, Tier: p.RTChannelTier_Bottom}
	emit("channel", &md)
	msg := r.RTMsg{Md: nc.Md, Mw: mw, Seq: 1, Sender: &user, InsertTime: 1700000000001}
	emit("message", &msg)
	inboxVersion := p.RTInboxVersion(5)
	inboxChannel := r.RTInboxChannel{Md: md, InboxVersion: 4, ReadThrough: 1, Hidden: false, Muted: true}
	inboxDelta := r.RTInboxDelta{InboxVersion: inboxVersion, AppID: p.RTAppID_Chat, Channels: []r.RTInboxChannel{inboxChannel}}
	pollResult := p.RTInboxPollRes{Bumped: true, InboxVersion: inboxVersion}
	cases := []struct {
		position rpc.Position
		arg      interface{}
		result   interface{}
	}{
		{0, (&r.RtNewChannelArg{Md: md, SetVers: 1}).Export(), nil},
		{2, (&r.RtListAllChannelsForTeamArg{Team: team, AppID: p.RTAppID_Chat, Last: 0}).Export(), (&r.RTChannelSet{Vers: 1, Lst: []r.RTChannelMetadata{md}, Mtime: 1700000000001}).Export()},
		{3, (&r.RtSendArg{Rtarg: r.RTSendArg{Md: nc.Md, Chid: chid.Short(), Mw: mw}}).Export(), (&r.RTSendRes{Seq: 1, InsertTime: 1700000000001}).Export()},
		{4, (&r.RtGetThreadArg{Q: r.RTThreadQuery{ChannelID: chid, Bookends: []r.RTThreadRangeBookends{{Start: 3, End: 1}}, Seqs: []p.RTMsgSeq{1, 2}}}).Export(), (&r.RTThreadPage{RangeMsgs: []r.RTMsgList{{Lst: []r.RTMsg{msg}}}, SeqMsgs: []r.RTMsg{msg}}).Export()},
		{5, (&r.RtGetInboxVersionArg{Key: r.RTInboxKey{AppID: p.RTAppID_Chat}}).Export(), inboxVersion.Export()},
		{6, (&r.RtGetChangedThreadsArg{Rtarg: r.RTGetChangedThreadsArg{AppID: p.RTAppID_Chat, Since: 3, Max: 100}}).Export(), inboxDelta.Export()},
		{7, (&r.RtReadThroughArg{Rtarg: r.RTReadThroughArg{ChannelID: chid, Seq: 1}}).Export(), nil},
		{8, (&r.RtPollInboxArg{Rtarg: r.RTPollInboxArg{AppID: p.RTAppID_Chat, Since: 4, Timeout: p.DurationMilli(25000)}}).Export(), pollResult.Export()},
		{9, (&r.RTSelectVhost{Host: host}).Export(), nil},
		{10, (&r.RtGetThreadRecentsArg{Ch: chid, StopAt: 0, Lim: 100}).Export(), (&r.RTMsgList{Lst: []r.RTMsg{msg}}).Export()},
	}
	for _, c := range cases {
		req, e := rpcRequestFrameAt(r.RealTimeProtocolID, c.position, c.arg, 7)
		if e != nil {
			t.Fatal(e)
		}
		put(fmt.Sprintf("request-%d.frame", c.position), req)
		res, e := rpcResponseFrame(r.RealTimeProtocolID, c.position, 7, c.result)
		if e != nil {
			t.Fatal(e)
		}
		put(fmt.Sprintf("response-%d.frame", c.position), res)
	}
}
