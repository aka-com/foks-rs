package drift

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/aka-com/foks-rs/tools/foks-protocol-sync/internal/model"
)

func shapeArtifact(shapes ...model.StructShape) model.ShapeArtifact {
	return model.ShapeArtifact{
		SchemaVersion: model.ShapeSchemaVersion,
		Source:        model.SourceIdentity{Module: "github.com/foks-proj/go-foks", Version: "v0.1.9"},
		Structs:       shapes,
	}
}

func shape(name, goFile string, fields ...string) model.StructShape {
	result := model.StructShape{Name: name, GoFile: goFile}
	for _, field := range fields {
		fieldName, fieldType, _ := strings.Cut(field, " ")
		result.Fields = append(result.Fields, model.StructField{Name: fieldName, Type: fieldType})
	}
	return result
}

func kinds(report Report) map[string]Class {
	result := make(map[string]Class)
	for _, change := range report.Changes {
		result[change.Kind+" "+change.Subject] = change.Class
	}
	return result
}

func TestCompareShapesClassifiesStructGrowthAndChanges(t *testing.T) {
	baseline := shapeArtifact(
		shape("rem.RTSendRes", "proto/rem/realtime.go", "Seq *lib.RTMsgSeqInternal__", "InsertTime *lib.TimeInternal__"),
		shape("rem.RTChannelSet", "proto/rem/realtime.go", "Vers *uint64", "Lst *[]X", "Mtime *uint64"),
		shape("lib.RTMsgMetadata", "proto/lib/realtime.go", "MsgID *RTMsgIDInternal__", "PrevID *RTMsgIDInternal__"),
		shape("lib.KVRoot", "proto/lib/kv.go", "Root *KVDirIDInternal__", "Vers *uint64"),
		shape("rem.Gone", "proto/rem/realtime.go", "A *uint64"),
		shape("rem.Outside", "proto/rem/unmapped.go", "A *uint64"),
	)
	candidate := shapeArtifact(
		// Appended, listed as tolerant: additive.
		shape("rem.RTSendRes", "proto/rem/realtime.go", "Seq *lib.RTMsgSeqInternal__", "InsertTime *lib.TimeInternal__", "WasReplay *bool"),
		// Unchanged.
		shape("rem.RTChannelSet", "proto/rem/realtime.go", "Vers *uint64", "Lst *[]X", "Mtime *uint64"),
		// Appended to a struct Rust decodes exactly: review.
		shape("lib.RTMsgMetadata", "proto/lib/realtime.go", "MsgID *RTMsgIDInternal__", "PrevID *RTMsgIDInternal__", "Extra *bool"),
		// Retyped field: breaking.
		shape("lib.KVRoot", "proto/lib/kv.go", "Root *KVDirIDInternal__", "Vers *uint32"),
		shape("rem.Outside", "proto/rem/unmapped.go", "B *uint64"),
		shape("rem.New", "proto/rem/realtime.go", "A *uint64"),
	)
	baseArtifact, candidateArtifact := fixtureArtifact(), fixtureArtifact()
	baseArtifact.Protocols[0].GoFile = "proto/rem/realtime.go"
	candidateArtifact.Protocols[0].GoFile = "proto/rem/realtime.go"
	policy := Policy{
		Protocols: map[string]string{"Probe": "Probe"},
		Supported: map[string]bool{"Probe.probe": true},
		Tolerant:  map[string]string{"rem.RTSendRes": "retained", "rem.Misspelt": "read_only"},
	}
	report := CompareWithShapes(baseArtifact, candidateArtifact, &ShapePair{Baseline: baseline, Candidate: candidate}, policy)
	got := kinds(report)
	want := map[string]Class{
		"struct_grown_tolerated rem.RTSendRes": Additive,
		"struct_grown lib.RTMsgMetadata":       BehaviorReviewRequired,
		"struct_changed lib.KVRoot":            WireBreaking,
		"struct_removed rem.Gone":              BehaviorReviewRequired,
		"struct_added rem.New":                 Additive,
		"tolerant_struct_unknown rem.Misspelt": BehaviorReviewRequired,
		// rem/unmapped.go has no protocol owner, so its source counts as
		// relevant (see sourceRelevant) and a changed field is breaking.
		"struct_changed rem.Outside": WireBreaking,
	}
	if len(got) != len(want) {
		t.Fatalf("changes = %#v", report.Changes)
	}
	for key, class := range want {
		if got[key] != class {
			t.Fatalf("%s: class %q, want %q; changes = %#v", key, got[key], class, report.Changes)
		}
	}
	for _, change := range report.Changes {
		if change.Subject == "rem.RTSendRes" && !strings.Contains(change.Detail, "2 -> 3 fields; appended WasReplay") {
			t.Fatalf("growth detail = %q", change.Detail)
		}
	}
	// Without shapes the comparison is unchanged.
	if report := Compare(baseArtifact, candidateArtifact, policy); len(report.Changes) != 0 {
		t.Fatalf("changes without shapes = %#v", report.Changes)
	}
}

func TestCompareShapesKeepsUnsupportedRemoteGrowthOutsideSlice(t *testing.T) {
	baseline := shapeArtifact(shape("rem.Beacon", "proto/rem/beacon.go", "A *uint64"))
	candidate := shapeArtifact(shape("rem.Beacon", "proto/rem/beacon.go", "A *uint64", "B *uint64"))
	base, next := fixtureArtifact(), fixtureArtifact()
	for _, artifact := range []*model.Artifact{&base, &next} {
		artifact.Protocols = append(artifact.Protocols, model.Protocol{
			Name: "Beacon", UniqueID: 2, GoFile: "proto/rem/beacon.go",
			Methods: []model.Method{{Name: "lookup", Position: 1, QualifiedName: "Beacon.lookup", ResultType: "void"}},
		})
	}
	report := CompareWithShapes(base, next, &ShapePair{Baseline: baseline, Candidate: candidate}, Policy{
		Protocols: map[string]string{"Probe": "Probe"},
		Supported: map[string]bool{"Probe.probe": true},
	})
	if got := kinds(report); len(got) != 1 || got["struct_grown rem.Beacon"] != OutsideLocalSlice {
		t.Fatalf("changes = %#v", report.Changes)
	}
}

func TestReadPolicyRecordsTolerantStructs(t *testing.T) {
	policy, err := ReadPolicy("../../../../crates/foks-server/protocol/policy-v1.toml")
	if err != nil {
		t.Fatal(err)
	}
	if policy.Tolerant["rem.RTSendRes"] != "retained" || policy.Tolerant["rem.RTChannelSet"] != "read_only" {
		t.Fatalf("tolerant structs = %v", policy.Tolerant)
	}
	// Every listed struct exists in the pinned struct-shape artifact, so no
	// entry can be misspelt into a no-op.
	shapes, err := model.ReadShapeArtifact("../../../../crates/foks-server/protocol/upstream-v0.1.9-structs.json")
	if err != nil {
		t.Fatal(err)
	}
	known := shapes.Shapes()
	for name := range policy.Tolerant {
		if _, ok := known[name]; !ok {
			t.Errorf("tolerant struct %s is not in the pinned shapes", name)
		}
	}
	if shape := known["rem.RTSendRes"]; len(shape.Fields) != 2 {
		t.Fatalf("pinned RTSendRes = %#v", shape)
	}
}

func TestReadPolicyRejectsUnknownStructDecoding(t *testing.T) {
	data, err := os.ReadFile("../../../../crates/foks-server/protocol/policy-v1.toml")
	if err != nil {
		t.Fatal(err)
	}
	text := strings.Replace(string(data), `decoding = "retained"`, `decoding = "exact"`, 1)
	path := filepath.Join(t.TempDir(), "policy.toml")
	if err := os.WriteFile(path, []byte(text), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := ReadPolicy(path); err == nil || !strings.Contains(err.Error(), "decoding") {
		t.Fatalf("expected decoding rejection, got %v", err)
	}
}
