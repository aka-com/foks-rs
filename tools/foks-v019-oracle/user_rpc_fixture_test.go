package main

import (
	"bytes"
	"os"
	"path/filepath"
	"testing"

	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/proto/rem"
)

const checkedUserFixtureDirectory = "../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/user"

// TestCheckedUserRPCFixtures exercises generated argument wrappers, not just
// the lower-level field encodings. Set FOKS_UPDATE_RPC_FIXTURES=1 to refresh
// these narrowly scoped frames without replacing the randomized user corpus.
func TestCheckedUserRPCFixtures(t *testing.T) {
	var chain rem.TeamChain
	chainBytes, err := os.ReadFile(filepath.Join(checkedUserFixtureDirectory, "team-chain.snowp"))
	if err != nil {
		t.Fatal(err)
	}
	if err := core.DecodeFromBytes(&chain, chainBytes); err != nil {
		t.Fatalf("decode checked team chain: %v", err)
	}
	if len(chain.Links) == 0 {
		t.Fatal("checked team chain is empty")
	}
	change, _, err := core.OpenGroupChange(&chain.Links[0])
	if err != nil {
		t.Fatalf("open checked team eldest: %v", err)
	}
	hostID := change.Entity.Host
	frame, err := rpcRequestFrameAt(
		rem.MerkleQueryProtocolID,
		2,
		(&rem.GetCurrentRootArg{HostID: &hostID}).Export(),
		1,
	)
	if err != nil {
		t.Fatalf("encode current-root request: %v", err)
	}
	path := filepath.Join(checkedUserFixtureDirectory, "merkle-current-root-request.frame")
	if os.Getenv("FOKS_UPDATE_RPC_FIXTURES") == "1" {
		if err := os.WriteFile(path, frame, 0o644); err != nil {
			t.Fatalf("update current-root request fixture: %v", err)
		}
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(frame, want) {
		t.Fatal("checked current-root request does not use the generated v0.1.9 argument wrapper")
	}
}
