package extract

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/aka-proj/foks-protocol-sync/internal/model"
)

func TestModuleExtractsStructuralIdentitiesAndAliases(t *testing.T) {
	root := fixtureModule(t, `package rem
const methodPosition = +0x2
var RenamedProtocolID rpc.ProtocolUniqueID = rpc.ProtocolUniqueID(0xc5884ff6)
func call() { rpc.NewMethodV2(RenamedProtocolID, methodPosition, "Renamed.run") }
func protocol() rpc.ProtocolV2 { return rpc.ProtocolV2{Name: "Renamed", ID: RenamedProtocolID, Methods: map[rpc.Position]rpc.ServeHandlerDescriptionV2{methodPosition: {Name: "run"}}} }
`)
	artifact, err := Module(root, model.SourceIdentity{Module: "example", Version: "test"})
	if err != nil {
		t.Fatal(err)
	}
	if got := artifact.Protocols[0]; got.Name != "Renamed" || got.UniqueID != 0xc5884ff6 || got.Methods[0].Position != 2 {
		t.Fatalf("unexpected protocol: %#v", got)
	}
	if got := artifact.Statuses; len(got) != 2 || got[1].Name != "SECOND" || got[1].Value != 16 {
		t.Fatalf("unexpected aliased statuses: %#v", got)
	}
	if len(artifact.Sources) != 2 {
		t.Fatalf("got %d source hashes, want 2", len(artifact.Sources))
	}
}

func TestModuleDiscoversEveryGeneratedRemoteProtocol(t *testing.T) {
	root := fixtureModule(t, `package rem
var RenamedProtocolID rpc.ProtocolUniqueID = rpc.ProtocolUniqueID(1)
func call() { rpc.NewMethodV2(RenamedProtocolID, 1, "Renamed.one") }
func protocol() rpc.ProtocolV2 { return rpc.ProtocolV2{Name: "Renamed", ID: RenamedProtocolID, Methods: map[rpc.Position]rpc.ServeHandlerDescriptionV2{1: {Name: "one"}}} }
`)
	writeFixture(t, root, "proto/rem/added.go", `package rem
var AddedProtocolID rpc.ProtocolUniqueID = rpc.ProtocolUniqueID(2)
func addedCall() { rpc.NewMethodV2(AddedProtocolID, 4, "Added.run") }
func addedProtocol() rpc.ProtocolV2 { return rpc.ProtocolV2{Name: "Added", ID: AddedProtocolID, Methods: map[rpc.Position]rpc.ServeHandlerDescriptionV2{4: {Name: "run"}}} }
`)
	writeFixture(t, root, "proto-src/rem/added.snowp", "struct Added {}\n")
	artifact, err := Module(root, model.SourceIdentity{Module: "example", Version: "test"})
	if err != nil {
		t.Fatal(err)
	}
	if len(artifact.Protocols) != 2 || artifact.Protocols[0].Name != "Added" {
		t.Fatalf("new protocol file was not discovered: %#v", artifact.Protocols)
	}
	if len(artifact.Sources) != 3 || artifact.Sources[1].Path != "proto-src/rem/added.snowp" {
		t.Fatalf("new Snowpack file was not discovered: %#v", artifact.Sources)
	}
}

func TestModuleRejectsDuplicatePositions(t *testing.T) {
	root := fixtureModule(t, `package rem
var RenamedProtocolID rpc.ProtocolUniqueID = rpc.ProtocolUniqueID(1)
func call() {
  rpc.NewMethodV2(RenamedProtocolID, 1, "Renamed.one")
  rpc.NewMethodV2(RenamedProtocolID, 1, "Renamed.two")
}
func protocol() rpc.ProtocolV2 { return rpc.ProtocolV2{Name: "Renamed", ID: RenamedProtocolID, Methods: map[rpc.Position]rpc.ServeHandlerDescriptionV2{1: {Name: "one"}}} }
`)
	_, err := Module(root, model.SourceIdentity{Module: "example", Version: "test"})
	if err == nil || (!strings.Contains(err.Error(), "position") && !strings.Contains(err.Error(), "client/handler")) {
		t.Fatalf("expected duplicate-position rejection, got %v", err)
	}
}

func TestModuleRejectsOverflow(t *testing.T) {
	root := fixtureModule(t, `package rem
var RenamedProtocolID rpc.ProtocolUniqueID = rpc.ProtocolUniqueID(0x100000000)
func call() { rpc.NewMethodV2(RenamedProtocolID, 1, "Renamed.one") }
func protocol() rpc.ProtocolV2 { return rpc.ProtocolV2{Name: "Renamed", ID: RenamedProtocolID, Methods: map[rpc.Position]rpc.ServeHandlerDescriptionV2{1: {Name: "one"}}} }
`)
	_, err := Module(root, model.SourceIdentity{Module: "example", Version: "test"})
	if err == nil || !strings.Contains(err.Error(), "invalid RenamedProtocolID") {
		t.Fatalf("expected overflow rejection, got %v", err)
	}
}

func TestModuleRejectsUnknownIntegerExpression(t *testing.T) {
	root := fixtureModule(t, `package rem
var RenamedProtocolID rpc.ProtocolUniqueID = rpc.ProtocolUniqueID(unknownID)
func call() { rpc.NewMethodV2(RenamedProtocolID, 1, "Renamed.one") }
func protocol() rpc.ProtocolV2 { return rpc.ProtocolV2{Name: "Renamed", ID: RenamedProtocolID, Methods: map[rpc.Position]rpc.ServeHandlerDescriptionV2{1: {Name: "one"}}} }
`)
	_, err := Module(root, model.SourceIdentity{Module: "example", Version: "test"})
	if err == nil || !strings.Contains(err.Error(), "unknown integer alias") {
		t.Fatalf("expected malformed-expression rejection, got %v", err)
	}
}

func TestModuleRejectsClientHandlerMismatch(t *testing.T) {
	root := fixtureModule(t, `package rem
var RenamedProtocolID rpc.ProtocolUniqueID = rpc.ProtocolUniqueID(1)
func call() { rpc.NewMethodV2(RenamedProtocolID, 1, "Renamed.one") }
func protocol() rpc.ProtocolV2 { return rpc.ProtocolV2{Name: "Renamed", ID: RenamedProtocolID, Methods: map[rpc.Position]rpc.ServeHandlerDescriptionV2{1: {Name: "two"}}} }
`)
	_, err := Module(root, model.SourceIdentity{Module: "example", Version: "test"})
	if err == nil || !strings.Contains(err.Error(), "client/handler mismatch") {
		t.Fatalf("expected client/handler rejection, got %v", err)
	}
}

func TestModuleRejectsSymlinkedSourceDirectories(t *testing.T) {
	root := fixtureModule(t, `package rem
var RenamedProtocolID rpc.ProtocolUniqueID = rpc.ProtocolUniqueID(1)
func call() { rpc.NewMethodV2(RenamedProtocolID, 1, "Renamed.one") }
func protocol() rpc.ProtocolV2 { return rpc.ProtocolV2{Name: "Renamed", ID: RenamedProtocolID, Methods: map[rpc.Position]rpc.ServeHandlerDescriptionV2{1: {Name: "one"}}} }
`)
	if err := os.Rename(filepath.Join(root, "proto-src", "rem"), filepath.Join(root, "proto-src", "real-rem")); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink("real-rem", filepath.Join(root, "proto-src", "rem")); err != nil {
		t.Fatal(err)
	}
	_, err := Module(root, model.SourceIdentity{Module: "example", Version: "test"})
	if err == nil || !strings.Contains(err.Error(), "non-symlink directory") {
		t.Fatalf("expected symlink rejection, got %v", err)
	}
}

func TestSemanticSnowpackIgnoresOnlyCommentsAndFormatting(t *testing.T) {
	left := semanticSnowpack([]byte("struct X { // comment\n value @0: Text; }\n"))
	right := semanticSnowpack([]byte("struct X{/* other */value@0:Text;}"))
	if string(left) != string(right) {
		t.Fatalf("formatting changed semantic form: %q != %q", left, right)
	}
	changed := semanticSnowpack([]byte("struct X { value @1: Text; }"))
	if string(left) == string(changed) {
		t.Fatal("field position change was erased")
	}
	quoted := semanticSnowpack([]byte(`go:import "a // b" as x;`))
	if !strings.Contains(string(quoted), "a // b") {
		t.Fatalf("comment marker inside string was erased: %q", quoted)
	}
	if string(semanticSnowpack([]byte("field name"))) == string(semanticSnowpack([]byte("fieldname"))) {
		t.Fatal("token-separating whitespace was erased")
	}
}

func fixtureModule(t *testing.T, probe string) string {
	t.Helper()
	root := t.TempDir()
	writeFixture(t, root, "proto/rem/probe.go", probe)
	writeFixture(t, root, "proto/lib/status.go", `package lib
type StatusCode int
const (
  StatusCode_OK StatusCode = 0
  statusAlias StatusCode = 0x10
  StatusCode_SECOND StatusCode = statusAlias
)
`)
	writeFixture(t, root, "proto/lib/common.go", `package lib
type ServerType int
const (ServerType_None ServerType = 0; ServerType_Probe ServerType = 10)
`)
	writeFixture(t, root, "proto-src/lib/common.snowp", "// common\n")
	writeFixture(t, root, "proto-src/rem/probe.snowp", "// probe\n")
	return root
}

func writeFixture(t *testing.T, root, relative, contents string) {
	t.Helper()
	path := filepath.Join(root, filepath.FromSlash(relative))
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(contents), 0o644); err != nil {
		t.Fatal(err)
	}
}
