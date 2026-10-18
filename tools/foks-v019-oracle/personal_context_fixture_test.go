package main

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"testing"

	proto "github.com/foks-proj/go-foks/proto/lib"
)

// Optional qualification output after the unchanged pinned Go client completes
// its real native signup, provisioning, encrypted KV and replay flow. Export
// only the public leaf certificate and identities, never TLS/PUK private keys.
func writePersonalContextFixture(t *testing.T, chain [][]byte, uid proto.UID, credential proto.EntityID) {
	t.Helper()
	path := os.Getenv("FOKS_GO_PERSONAL_CONTEXT_PATH")
	if path == "" {
		return
	}
	if len(chain) == 0 || len(chain[0]) == 0 {
		t.Fatal("personal context requires the actually used leaf certificate")
	}
	fixture := struct {
		Schema         int    `json:"schema"`
		CertificateDER []byte `json:"certificate_der"`
		UID            string `json:"uid"`
		CredentialID   string `json:"credential_id"`
	}{1, chain[0], hex.EncodeToString(uid.Bytes()), hex.EncodeToString(credential.Bytes())}
	file, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		t.Fatal(err)
	}
	if err = json.NewEncoder(file).Encode(fixture); err != nil {
		_ = file.Close()
		t.Fatal(err)
	}
	if err = file.Close(); err != nil {
		t.Fatal(err)
	}
}
