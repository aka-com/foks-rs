package main

import (
	"bytes"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/foks-proj/go-foks/lib/core"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
	"github.com/foks-proj/go-snowpack-rpc/rpc"
)

func TestFixtureProtocolsUseGeneratedDescriptors(t *testing.T) {
	protocols := []rpc.ProtocolUniqueID{
		rem.BeaconProtocolID,
		rem.KexProtocolID,
		rem.KVStoreProtocolID,
		rem.MerkleQueryProtocolID,
		rem.ProbeProtocolID,
		rem.RegProtocolID,
		rem.TeamAdminProtocolID,
		rem.TeamGuestProtocolID,
		rem.TeamLoaderProtocolID,
		rem.TeamMemberProtocolID,
		rem.UserProtocolID,
	}
	for _, id := range protocols {
		definition, err := generatedProtocol(id)
		if err != nil {
			t.Fatalf("protocol 0x%x: %v", id, err)
		}
		if definition.ID != id || definition.Name == "" || len(definition.Methods) == 0 {
			t.Fatalf("invalid generated descriptor for protocol 0x%x: %#v", id, definition)
		}
		for position, method := range definition.Methods {
			argument := method.MakeArg()
			if argument == nil || reflect.TypeOf(argument).Kind() != reflect.Pointer {
				t.Fatalf("%s.%s @%d generated invalid argument %T", definition.Name, method.Name, position, argument)
			}
		}
	}
}

func TestGeneratedWireArgumentSelectsExactEnvelope(t *testing.T) {
	probe := (&rem.ProbeArg{Hostname: "foks.app"}).Export()
	wrapped, err := generatedWireArgument(rem.ProbeProtocolID, 1, probe)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := wrapped.(*rpc.DataWrap[proto.Header, *rem.ProbeArgInternal__]); !ok {
		t.Fatalf("Probe.probe generated %T, expected DataWrap", wrapped)
	}

	teamLoad := (&rem.LoadTeamChainArg{}).Export()
	bare, err := generatedWireArgument(rem.TeamLoaderProtocolID, 3, teamLoad)
	if err != nil {
		t.Fatal(err)
	}
	if bare != teamLoad {
		t.Fatalf("TeamLoader.loadTeamChain generated %T, expected bare %T", bare, teamLoad)
	}

	if _, err := generatedWireArgument(rem.TeamLoaderProtocolID, 3, probe); err == nil {
		t.Fatal("accepted the argument type for a different generated method")
	}
	if _, err := generatedWireArgument(rem.ProbeProtocolID, 99, probe); err == nil {
		t.Fatal("accepted an unknown generated method position")
	}
}

func TestCheckedProbeRequestUsesGeneratedDescriptor(t *testing.T) {
	got, err := probeRequestFrame(proto.TCPAddr("foks.app:4430"))
	if err != nil {
		t.Fatal(err)
	}
	want, err := os.ReadFile(filepath.Join(
		"../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app",
		"probe-request.frame",
	))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want) {
		t.Fatal("checked probe request differs from the generated v0.1.9 descriptor")
	}
}

func TestOfflineGeneratorsUseGeneratedMethodArguments(t *testing.T) {
	address := proto.TCPAddr("foks.app:4430")
	fixture, err := os.ReadFile(filepath.Join(
		"../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app",
		"probe-response.snowp",
	))
	if err != nil {
		t.Fatal(err)
	}
	var response rem.ProbeRes
	if err := core.DecodeFromBytes(&response, fixture); err != nil {
		t.Fatal(err)
	}
	chain, err := core.PlayChain(address, response.Hostchain, nil)
	if err != nil {
		t.Fatal(err)
	}
	root, err := verifyMerkleRoot(chain, response.MerkleRoot)
	if err != nil {
		t.Fatal(err)
	}
	if err := writeUserFixtures(t.TempDir(), address, chain.HostID(), root); err != nil {
		t.Fatalf("user fixture generator: %v", err)
	}
	if err := writeSignupFixtures(t.TempDir(), chain.HostID(), root); err != nil {
		t.Fatalf("signup fixture generator: %v", err)
	}
}
