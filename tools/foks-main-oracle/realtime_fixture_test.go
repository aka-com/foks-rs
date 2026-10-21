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
	"github.com/keybase/go-codec/codec"
)

// Realtime values as go-foks main (35dc037) puts them on the wire: the grown
// RTSendRes (b41b354) and RTChannelMetadata (c334c97), the payload-free
// RT_MSG_REPLAY_MISMATCH status, and pegged Reply/Reactji/Edit bodies sealed
// the way client/librt sealMessage seals them (bf9ace5). Identifiers and keys
// match the v0.1.9 realtime fixtures so the Rust side can open the messages.
//
// Set FOKS_RT_MAIN_FIXTURE_OUT to write the files; otherwise the test compares
// against the checked-in tree.
func TestRealtimeMainFixtures(t *testing.T) {
	root := os.Getenv("FOKS_RT_MAIN_FIXTURE_OUT")
	generate := root != ""
	if !generate {
		root = filepath.Join("..", "..", "crates", "foks-snowpack", "tests", "fixtures", "foks-main", "realtime")
	} else if err := os.MkdirAll(root, 0755); err != nil {
		t.Fatal(err)
	}
	put := func(name string, b []byte) {
		t.Helper()
		path := filepath.Join(root, name)
		if generate {
			if e := os.WriteFile(path, b, 0644); e != nil {
				t.Fatal(e)
			}
			return
		}
		old, e := os.ReadFile(path)
		if e != nil {
			t.Fatal(e)
		}
		if !bytes.Equal(b, old) {
			t.Fatalf("fixture changed: %s", name)
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
	frame := func(name string, content []interface{}) {
		t.Helper()
		handle := core.Codec()
		var body []byte
		if e := codec.NewEncoderBytes(&body, handle).Encode(content); e != nil {
			t.Fatal(e)
		}
		var length []byte
		if e := codec.NewEncoderBytes(&length, handle).Encode(len(body)); e != nil {
			t.Fatal(e)
		}
		put(name, append(length, body...))
	}
	// Request and response envelopes as RealTimeClient builds them with
	// MakeArgHeader = core.MakeProtoHeader; the server's response wrapper
	// carries the same header.
	request := func(name string, position rpc.Position, data interface{}) {
		t.Helper()
		arg := &rpc.DataWrap[p.Header, interface{}]{Header: core.MakeProtoHeader(), Data: data}
		frame(name, []interface{}{rpc.MethodCallV2, rpc.SeqNumber(7), r.RealTimeProtocolID, position, arg})
	}
	response := func(name string, data interface{}) {
		t.Helper()
		res := &rpc.DataWrap[p.Header, interface{}]{Header: core.MakeProtoHeader(), Data: data}
		frame(name, []interface{}{rpc.MethodResponse, rpc.SeqNumber(7), (*p.Status)(nil), res})
	}

	var seed p.SecretSeed32
	for i := range seed {
		seed[i] = byte(i)
	}
	put("seed.bin", seed[:])
	derive := func(seed p.SecretSeed32, obj core.CryptoPayloader) p.SecretSeed32 {
		t.Helper()
		k, e := core.GenericDeriveKey32(seed, obj)
		if e != nil {
			t.Fatal(e)
		}
		return *k
	}
	kd := p.NewKeyDerivationDefault(p.KeyDerivationType_AppKey)
	app := derive(seed, &kd)
	ad := p.NewAppKeyDerivationWithEnum(p.AppKeyEnum_Realtime)
	rt := derive(app, &ad)
	keyFor := func(typ p.RTKeyType) p.SecretBoxKey {
		d := p.RTKeyDerivation{App: p.RTAppID_Chat, Var: p.NewRTKeyVarDefault(typ)}
		return p.SecretBoxKey(derive(rt, &d))
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

	var partial p.NaclNonce
	for i := range partial {
		partial[i] = byte(i + 16)
	}
	name := p.NewRTChannelNamePlaintextWithUtf8v1(p.RTChannelName(""))
	nameKey := keyFor(p.RTKeyType_ChannelName)
	nameCt, e := core.SealIntoSecretBoxWithNonce(&name, &partial, &nameKey)
	if e != nil {
		t.Fatal(e)
	}
	nameBox := p.NewSecretBoxWithNacl(p.NaclSecretBox{Nonce: partial, Ciphertext: nameCt})
	rg := p.RoleAndGen{Role: p.DefaultRole, Gen: 1}
	md := r.RTChannelMetadata{
		Id: chid, ParentTeam: team, AppID: p.RTAppID_Chat, Seqno: 1,
		NameBox: p.RTBoxRG{Rg: p.RoleAndGen{Role: p.MinRTRole, Gen: 1}, Box: nameBox},
		Roles:   p.RolePair{Read: p.DefaultRole, Write: p.DefaultRole},
		Ctime:   1700000000000, Mtime: 1700000000001, UpdatedAt: 1,
		Tier:    p.RTChannelTier_Bottom, NoPush: true,
	}
	emit("channel-no-push", &md)
	// A Go-main client always writes all fourteen fields, false included.
	pushing := md
	pushing.NoPush = false
	emit("channel-push", &pushing)
	request("request-0-no-push.frame", 0, (&r.RtNewChannelArg{Md: md, SetVers: 1}).Export())
	response("response-2-no-push.frame", (&r.RTChannelSet{Vers: 1, Lst: []r.RTChannelMetadata{md}, Mtime: 1700000000001}).Export())

	fresh := r.RTSendRes{Seq: 1, InsertTime: 1700000000001}
	replay := r.RTSendRes{Seq: 1, InsertTime: 1700000000001, WasReplay: true}
	emit("send-res-fresh", &fresh)
	emit("send-res-replay", &replay)
	response("response-3-fresh.frame", fresh.Export())
	response("response-3-replay.frame", replay.Export())
	status := core.ErrorToStatus(core.RTMsgReplayMismatchError{})
	frame("status-12007.frame", []interface{}{rpc.MethodResponse, rpc.SeqNumber(7), status.Export(), nil})

	// Pegged messages: target is the v0.1.9 fixture's message 0x33..., and
	// each pegged message names the previous one as its predecessor.
	var target p.RTMsgID
	target[0] = 0x33
	dataKey := keyFor(p.RTKeyType_Data)
	sender := p.FQParty{Party: user, Host: host}
	prev := target
	pegged := []struct {
		name string
		typ  p.RTMsgType
		body []byte
	}{
		{"reply", p.RTMsgType_Reply, []byte("a reply")},
		{"reactji", p.RTMsgType_Reactji, []byte(`{"emoji":"👍"}`)},
		{"edit", p.RTMsgType_Edit, []byte("hello (edited)")},
		{"delete", p.RTMsgType_Edit, []byte{}},
	}
	for i, c := range pegged {
		var mid p.RTMsgID
		mid[0] = 0x40 + byte(i)
		seq := p.RTMsgSeq(i + 2)
		meta := p.RTMsgMetadata{MsgID: mid, PrevID: prev, PrevSeq: seq - 1, SendTime: p.Time(1700000000100 + i), Typ: c.typ}
		nc := p.RTMsgNoncer{Md: meta, Sender: &sender, AppID: p.RTAppID_Chat, Team: p.FQParty{Party: p.PartyID(team[:]), Host: host}, Chid: chid}
		hash, e := core.PrefixedHash(&nc)
		if e != nil {
			t.Fatal(e)
		}
		nonce := hash.DomainSeparatedNaclNonce()
		pg := p.RTMsgPlaintextPegged{Basic: p.RTMsgPlaintextBasic(c.body), ReplyTo: target}
		var body p.RTMsgBody
		switch c.typ {
		case p.RTMsgType_Reply:
			body = p.NewRTMsgBodyWithReply(pg)
		case p.RTMsgType_Reactji:
			body = p.NewRTMsgBodyWithReactji(pg)
		default:
			body = p.NewRTMsgBodyWithEdit(pg)
		}
		emit(fmt.Sprintf("pegged-%s-body", c.name), &body)
		ct, e := core.SealIntoSecretBoxWithDomainSeparatedNonceAndPadding(&body, nonce, &dataKey, 0)
		if e != nil {
			t.Fatal(e)
		}
		mw := p.NewRTMsgWrapperWithEncrypted(p.RTMsgBox{Ctext: p.NewRTMsgCiphertextWithNacl(ct), Rg: rg})
		msg := r.RTMsg{Md: meta, Mw: mw, Seq: seq, Sender: &user, InsertTime: p.Time(1700000000200 + i)}
		emit(fmt.Sprintf("pegged-%s", c.name), &msg)
		prev = mid
	}
}
