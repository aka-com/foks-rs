package model

import (
	"path/filepath"
	"testing"
)

func TestShapeArtifactRoundTripsAndValidates(t *testing.T) {
	artifact := ShapeArtifact{
		Source: SourceIdentity{Module: "example", Version: "test"},
		Structs: []StructShape{
			{Name: "rem.B", GoFile: "proto/rem/b.go", Fields: []StructField{{Name: "X", Type: "*uint64"}}},
			{Name: "lib.A", GoFile: "proto/lib/a.go", Fields: []StructField{{Name: "Y", Type: "*bool"}}},
		},
	}
	path := filepath.Join(t.TempDir(), "shapes.json")
	if err := WriteShapeArtifact(path, artifact); err != nil {
		t.Fatal(err)
	}
	read, err := ReadShapeArtifact(path)
	if err != nil {
		t.Fatal(err)
	}
	if read.SchemaVersion != ShapeSchemaVersion || read.Structs[0].Name != "lib.A" {
		t.Fatalf("read = %#v", read)
	}
	for _, bad := range []StructShape{
		{Name: "proto.A", Fields: []StructField{{Name: "X", Type: "T"}}},
		{Name: "lib.A"},
		{Name: "lib.A", Fields: []StructField{{Name: "X", Type: "T"}, {Name: "X", Type: "U"}}},
	} {
		invalid := ShapeArtifact{SchemaVersion: ShapeSchemaVersion, Structs: []StructShape{bad}}
		if invalid.Validate() == nil {
			t.Fatalf("accepted %#v", bad)
		}
	}
	duplicate := ShapeArtifact{SchemaVersion: ShapeSchemaVersion, Structs: []StructShape{artifact.Structs[0], artifact.Structs[0]}}
	if duplicate.Validate() == nil {
		t.Fatal("accepted a duplicate struct")
	}
}
