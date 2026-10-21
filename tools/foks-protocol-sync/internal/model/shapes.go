package model

import (
	"encoding/json"
	"fmt"
	"os"
	"sort"
	"strings"
)

// ShapeSchemaVersion versions the struct-shape artifact. It is separate from
// the protocol artifact so that recording shapes does not change the pinned
// protocol metadata or its digest.
const ShapeSchemaVersion = 1

// StructField is one positional field of a generated go-foks struct, in wire
// order. Type is the Go type of the generated `Internal__` field.
type StructField struct {
	Name string `json:"name"`
	Type string `json:"type"`
}

// StructShape is the positional layout of one generated struct: go-codec
// encodes the `Internal__` form as an array of its fields in declaration
// order, so the field list is the wire layout. Name is `<package>.<Type>`,
// for example `rem.RTSendRes`.
type StructShape struct {
	Name   string        `json:"name"`
	GoFile string        `json:"go_file"`
	Fields []StructField `json:"fields"`
}

type ShapeArtifact struct {
	SchemaVersion int            `json:"schema_version"`
	Source        SourceIdentity `json:"source"`
	Structs       []StructShape  `json:"structs"`
}

func (a *ShapeArtifact) Normalize() {
	a.SchemaVersion = ShapeSchemaVersion
	sort.Slice(a.Structs, func(i, j int) bool { return a.Structs[i].Name < a.Structs[j].Name })
}

func (a ShapeArtifact) Validate() error {
	if a.SchemaVersion != ShapeSchemaVersion {
		return fmt.Errorf("unsupported shape artifact schema %d", a.SchemaVersion)
	}
	names := make(map[string]bool)
	for _, shape := range a.Structs {
		pkg, typ, ok := strings.Cut(shape.Name, ".")
		if !ok || (pkg != "lib" && pkg != "rem") || typ == "" || names[shape.Name] {
			return fmt.Errorf("duplicate or malformed struct name %q", shape.Name)
		}
		names[shape.Name] = true
		if len(shape.Fields) == 0 {
			return fmt.Errorf("struct %s has no fields", shape.Name)
		}
		fields := make(map[string]bool)
		for _, field := range shape.Fields {
			if field.Name == "" || field.Type == "" || fields[field.Name] {
				return fmt.Errorf("struct %s has a duplicate or empty field %q", shape.Name, field.Name)
			}
			fields[field.Name] = true
		}
	}
	return nil
}

// Shapes indexes the artifact by struct name.
func (a ShapeArtifact) Shapes() map[string]StructShape {
	result := make(map[string]StructShape, len(a.Structs))
	for _, shape := range a.Structs {
		result[shape.Name] = shape
	}
	return result
}

func ReadShapeArtifact(path string) (ShapeArtifact, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return ShapeArtifact{}, err
	}
	var artifact ShapeArtifact
	if err := json.Unmarshal(data, &artifact); err != nil {
		return ShapeArtifact{}, err
	}
	if err := artifact.Validate(); err != nil {
		return ShapeArtifact{}, err
	}
	return artifact, nil
}

func WriteShapeArtifact(path string, artifact ShapeArtifact) error {
	artifact.Normalize()
	if err := artifact.Validate(); err != nil {
		return err
	}
	data, err := json.MarshalIndent(artifact, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	return os.WriteFile(path, data, 0o644)
}
