// Package main holds fixture generators that run against an unreleased
// go-foks main revision. The pinned v0.1.9 generators live in
// tools/foks-v019-oracle and are not affected by this module.
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"github.com/foks-proj/go-foks/lib/core"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-snowpack-rpc/rpc"
	"github.com/keybase/go-codec/codec"
)

// fixtureDir compares emitted bytes with crates/foks-snowpack/tests/fixtures/
// foks-main/<area> by default. With FOKS_MAIN_FIXTURE_OUT set, it writes
// <FOKS_MAIN_FIXTURE_OUT>/<area> instead, including a SHA256SUMS file.
type fixtureDir struct {
	t        *testing.T
	root     string
	generate bool
	sums     map[string]string
}

func newFixtureDir(t *testing.T, area string) *fixtureDir {
	t.Helper()
	out := os.Getenv("FOKS_MAIN_FIXTURE_OUT")
	d := &fixtureDir{t: t, sums: make(map[string]string)}
	if out != "" {
		d.generate = true
		d.root = filepath.Join(out, area)
		if err := os.MkdirAll(d.root, 0o755); err != nil {
			t.Fatal(err)
		}
	} else {
		d.root = filepath.Join("..", "..", "crates", "foks-snowpack", "tests", "fixtures", "foks-main", area)
	}
	return d
}

func (d *fixtureDir) put(name string, b []byte) {
	d.t.Helper()
	sum := sha256.Sum256(b)
	d.sums[name] = hex.EncodeToString(sum[:])
	d.write(name, b)
}

// finish writes or checks SHA256SUMS in `sha256sum` format, sorted by name.
func (d *fixtureDir) finish() {
	d.t.Helper()
	names := make([]string, 0, len(d.sums))
	for name := range d.sums {
		names = append(names, name)
	}
	sort.Strings(names)
	var b strings.Builder
	for _, name := range names {
		b.WriteString(d.sums[name] + "  " + name + "\n")
	}
	d.write("SHA256SUMS", []byte(b.String()))
}

func (d *fixtureDir) write(name string, b []byte) {
	d.t.Helper()
	path := filepath.Join(d.root, name)
	if d.generate {
		if err := os.WriteFile(path, b, 0o644); err != nil {
			d.t.Fatal(err)
		}
		return
	}
	old, err := os.ReadFile(path)
	if err != nil {
		d.t.Fatal(err)
	}
	if !bytes.Equal(old, b) {
		d.t.Fatalf("fixture changed: %s", name)
	}
}

// encodeFrame length-prefixes one MessagePack RPC message the way
// go-snowpack-rpc frames it on the wire.
func encodeFrame(message []interface{}) ([]byte, error) {
	handle := core.Codec()
	var content []byte
	if err := codec.NewEncoderBytes(&content, handle).Encode(message); err != nil {
		return nil, err
	}
	var length []byte
	if err := codec.NewEncoderBytes(&length, handle).Encode(len(content)); err != nil {
		return nil, err
	}
	return append(length, content...), nil
}

// responseFrame encodes a successful response carrying the value a
// generated server handler returned.
func responseFrame(sequence rpc.SeqNumber, result interface{}) ([]byte, error) {
	return encodeFrame([]interface{}{rpc.MethodResponse, sequence, (*proto.Status)(nil), result})
}

// errorResponseFrame mirrors the server's error reply: the exported
// positional status in the error slot and a nil result.
func errorResponseFrame(sequence rpc.SeqNumber, status proto.Status) ([]byte, error) {
	return encodeFrame([]interface{}{rpc.MethodResponse, sequence, status.Export(), nil})
}
