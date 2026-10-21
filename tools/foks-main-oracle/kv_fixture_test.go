package main

import (
	"context"
	"testing"

	"github.com/foks-proj/go-foks/lib/core"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
	"github.com/foks-proj/go-snowpack-rpc/rpc"
	"github.com/keybase/go-codec/codec"
)

// kvReplayStub answers kvMkdir and kvPutSmallFileOrSymlink with a fixed
// wasReplay value; the generated protocol table calls nothing else here.
type kvReplayStub struct {
	rem.KVStoreInterface
	wasReplay bool
}

func (s kvReplayStub) KvMkdir(context.Context, rem.KvMkdirArg) (rem.KVMkdirRes, error) {
	return rem.KVMkdirRes{WasReplay: s.wasReplay}, nil
}

func (s kvReplayStub) KvPutSmallFileOrSymlink(context.Context, rem.KvPutSmallFileOrSymlinkArg) (rem.KVPutSmallFileOrSymlinkRes, error) {
	return rem.KVPutSmallFileOrSymlinkRes{WasReplay: s.wasReplay}, nil
}

func (kvReplayStub) CheckArgHeader(context.Context, proto.Header) error { return nil }
func (kvReplayStub) MakeResHeader() proto.Header                        { return core.MakeProtoHeader() }
func (kvReplayStub) ErrorWrapper() func(error) proto.Status             { return core.ErrorToStatus }

// TestKVReplayResultFixtures emits the KVMkdirRes and
// KVPutSmallFileOrSymlinkRes response frames go-foks main returns
// (fd03098, b3c9177), the void frame a v0.1.9 server returns for the same
// methods, and the KV_RACE_ERROR frames for a reused object ID.
func TestKVReplayResultFixtures(t *testing.T) {
	d := newFixtureDir(t, "kv")
	ctx := context.Background()
	const sequence = rpc.SeqNumber(1)

	serve := func(position rpc.Position, wasReplay bool) interface{} {
		t.Helper()
		method := rem.KVStoreProtocol(kvReplayStub{wasReplay: wasReplay}).Methods[position]
		var arg interface{}
		switch position {
		case 0:
			arg = &rpc.DataWrap[proto.Header, *rem.KvMkdirArgInternal__]{
				Header: core.MakeProtoHeader(), Data: (&rem.KvMkdirArg{}).Export(),
			}
		case 7:
			arg = &rpc.DataWrap[proto.Header, *rem.KvPutSmallFileOrSymlinkArgInternal__]{
				Header: core.MakeProtoHeader(), Data: (&rem.KvPutSmallFileOrSymlinkArg{}).Export(),
			}
		default:
			t.Fatalf("unexpected position %d", position)
		}
		result, err := method.Handler(ctx, arg)
		if err != nil {
			t.Fatal(err)
		}
		return result
	}
	frame := func(name string, result interface{}) {
		t.Helper()
		b, err := responseFrame(sequence, result)
		if err != nil {
			t.Fatal(err)
		}
		d.put(name, b)
	}
	if rem.KVStoreProtocol(kvReplayStub{}).Methods[7].Name != "kvPutSmallFileOrSymlink" {
		t.Fatal("kvPutSmallFileOrSymlink is not at position 7")
	}

	frame("mkdir-response-fresh.frame", serve(0, false))
	frame("mkdir-response-replay.frame", serve(0, true))
	frame("put-small-response-fresh.frame", serve(7, false))
	frame("put-small-response-replay.frame", serve(7, true))

	// A v0.1.9 void handler returns a DataWrap whose Data is nil; DataWrap
	// omits it, so only Header is on the wire.
	void := &rpc.DataWrap[proto.Header, interface{}]{Header: core.MakeProtoHeader()}
	frame("void-response.frame", void)

	race := func(name, message string) {
		t.Helper()
		b, err := errorResponseFrame(sequence, core.ErrorToStatus(core.KVRaceError(message)))
		if err != nil {
			t.Fatal(err)
		}
		d.put(name, b)
	}
	race("mkdir-race-response.frame", "dir id reused with different contents")
	race("put-small-race-response.frame", "small-file node id reused with different contents")

	// The generated main client reads the v0.1.9 void result as
	// wasReplay=false, and the struct results as encoded.
	decode := func(wrapped interface{}) bool {
		t.Helper()
		var b []byte
		if err := codec.NewEncoderBytes(&b, core.Codec()).Encode(wrapped); err != nil {
			t.Fatal(err)
		}
		var tmp rpc.DataWrap[proto.Header, rem.KVMkdirResInternal__]
		if err := codec.NewDecoderBytes(b, core.Codec()).Decode(&tmp); err != nil {
			t.Fatal(err)
		}
		return tmp.Data.Import().WasReplay
	}
	if decode(void) {
		t.Fatal("void result decoded as a replay")
	}
	if decode(serve(0, false)) || !decode(serve(0, true)) {
		t.Fatal("struct result did not round-trip")
	}
	d.finish()
}
