package main

import (
	"strings"
	"testing"

	"github.com/foks-proj/go-foks/lib/merkle"
	proto "github.com/foks-proj/go-foks/proto/lib"
)

func TestGoV019CannotHashEpoch65536BackPointers(t *testing.T) {
	sequence := merkle.MerkleBackpointerSequence(proto.MerkleEpno(65_536))
	if len(sequence) != 16 {
		t.Fatalf("epoch 65536 has %d back pointers, expected 16", len(sequence))
	}
	pointers := make(proto.MerkleBackPointers, len(sequence))
	for index, epoch := range sequence {
		pointers[index].Epno = epoch
	}
	var hash proto.MerkleBackPointerHash
	err := merkle.HashBackPointers(&pointers, &hash)
	if err == nil || !strings.Contains(err.Error(), "array16 where fixed array encoding is possible") {
		t.Fatalf("expected the v0.1.9 array16 canonicality failure, got %v", err)
	}
}
