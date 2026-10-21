package extract

import (
	"strings"
	"testing"

	"github.com/aka-com/foks-rs/tools/foks-protocol-sync/internal/model"
)

func TestShapesRecordPositionalStructsInWireOrder(t *testing.T) {
	root := fixtureModule(t, `package rem
var RenamedProtocolID rpc.ProtocolUniqueID = rpc.ProtocolUniqueID(1)
func call() { rpc.NewMethodV2(RenamedProtocolID, 1, "Renamed.one") }
func protocol() rpc.ProtocolV2 { return rpc.ProtocolV2{Name: "Renamed", ID: RenamedProtocolID, Methods: map[rpc.Position]rpc.ServeHandlerDescriptionV2{1: {Name: "one"}}} }
type SendRes struct { Seq lib.Seq; WasReplay bool }
type SendResInternal__ struct {
	_struct    struct{} `+"`codec:\",toarray\"`"+`
	Seq        *lib.SeqInternal__
	InsertTime, Mtime *lib.TimeInternal__
	WasReplay  *bool
}
type BodyInternal__ struct {
	_struct  struct{} `+"`codec:\",toarray\"`"+`
	T        *TypeInternal__
	Switch__ BodyInternalSwitch__
}
type BodyInternalSwitch__ struct {
	_struct struct{} `+"`codec:\",omitempty\"`"+`
	F_1__   *[]byte `+"`codec:\"1\"`"+`
}
type MapShapedInternal__ struct {
	A *uint64
}
`)
	writeFixture(t, root, "proto/lib/kv.go", `package lib
type KVUsageStatsInternal__ struct {
	_struct struct{} `+"`codec:\",toarray\"`"+`
	Num     *uint64
	Sum     *uint64
}
`)
	shapes, err := Shapes(root, model.SourceIdentity{Module: "example", Version: "test"})
	if err != nil {
		t.Fatal(err)
	}
	if len(shapes.Structs) != 2 {
		t.Fatalf("structs = %#v", shapes.Structs)
	}
	usage, send := shapes.Structs[0], shapes.Structs[1]
	if usage.Name != "lib.KVUsageStats" || usage.GoFile != "proto/lib/kv.go" || len(usage.Fields) != 2 {
		t.Fatalf("usage = %#v", usage)
	}
	var fields []string
	for _, field := range send.Fields {
		fields = append(fields, field.Name+":"+field.Type)
	}
	want := "Seq:*lib.SeqInternal__ InsertTime:*lib.TimeInternal__ Mtime:*lib.TimeInternal__ WasReplay:*bool"
	if send.Name != "rem.SendRes" || strings.Join(fields, " ") != want {
		t.Fatalf("send = %s %v", send.Name, fields)
	}
}

func TestShapesRejectDuplicateDefinitions(t *testing.T) {
	root := fixtureModule(t, `package rem
var RenamedProtocolID rpc.ProtocolUniqueID = rpc.ProtocolUniqueID(1)
func call() { rpc.NewMethodV2(RenamedProtocolID, 1, "Renamed.one") }
func protocol() rpc.ProtocolV2 { return rpc.ProtocolV2{Name: "Renamed", ID: RenamedProtocolID, Methods: map[rpc.Position]rpc.ServeHandlerDescriptionV2{1: {Name: "one"}}} }
type XInternal__ struct {
	_struct struct{} `+"`codec:\",toarray\"`"+`
	A       *uint64
}
`)
	writeFixture(t, root, "proto/rem/other.go", `package rem
type XInternal__ struct {
	_struct struct{} `+"`codec:\",toarray\"`"+`
	B       *uint64
}
`)
	if _, err := Shapes(root, model.SourceIdentity{Module: "example", Version: "test"}); err == nil || !strings.Contains(err.Error(), "rem.X is defined in") {
		t.Fatalf("expected duplicate rejection, got %v", err)
	}
}
