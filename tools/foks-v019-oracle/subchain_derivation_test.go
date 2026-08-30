package main

import (
	"encoding/hex"
	"testing"

	"github.com/foks-proj/go-foks/lib/core"
	proto "github.com/foks-proj/go-foks/proto/lib"
)

func TestSubchainLocationReference(t *testing.T) {
	var seed proto.TreeLocation
	for i := range seed {
		seed[i] = 0x35
	}
	location, err := core.SubchainTreeLocation(seed, proto.ChainType_TeamMembership)
	if err != nil {
		t.Fatal(err)
	}
	const expected = "90de9044fcee5b2884e0d04f7c71d6af83ce047b051f56c207b50bdb9d7f4096"
	if actual := hex.EncodeToString(location[:]); actual != expected {
		t.Fatalf("subchain location = %s, want %s", actual, expected)
	}
}
