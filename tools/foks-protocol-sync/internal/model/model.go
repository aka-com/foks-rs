package model

import (
	"encoding/json"
	"fmt"
	"os"
	"sort"
)

const SchemaVersion = 2

type SourceIdentity struct {
	Module   string `json:"module"`
	Version  string `json:"version"`
	Sum      string `json:"sum"`
	GoModSum string `json:"go_mod_sum"`
	Commit   string `json:"commit,omitempty"`
}

type Method struct {
	Name          string `json:"name"`
	Position      uint32 `json:"position"`
	QualifiedName string `json:"qualified_name"`
	ResultType    string `json:"result_type"`
}

type Protocol struct {
	Name           string   `json:"name"`
	UniqueID       uint32   `json:"unique_id"`
	GoFile         string   `json:"go_file"`
	ArgumentHeader bool     `json:"argument_header"`
	ResultHeader   bool     `json:"result_header"`
	Methods        []Method `json:"methods"`
}

type NamedValue struct {
	Name   string `json:"name"`
	Value  int64  `json:"value"`
	GoFile string `json:"go_file"`
}

type SourceFile struct {
	Path           string `json:"path"`
	SHA256         string `json:"sha256"`
	SemanticSHA256 string `json:"semantic_sha256"`
}

type Artifact struct {
	SchemaVersion int            `json:"schema_version"`
	Source        SourceIdentity `json:"source"`
	Protocols     []Protocol     `json:"protocols"`
	Statuses      []NamedValue   `json:"statuses"`
	Services      []NamedValue   `json:"services"`
	Sources       []SourceFile   `json:"sources"`
}

func (a *Artifact) Normalize() {
	a.SchemaVersion = SchemaVersion
	for i := range a.Protocols {
		sort.Slice(a.Protocols[i].Methods, func(j, k int) bool {
			if a.Protocols[i].Methods[j].Position != a.Protocols[i].Methods[k].Position {
				return a.Protocols[i].Methods[j].Position < a.Protocols[i].Methods[k].Position
			}
			return a.Protocols[i].Methods[j].Name < a.Protocols[i].Methods[k].Name
		})
	}
	sort.Slice(a.Protocols, func(i, j int) bool { return a.Protocols[i].Name < a.Protocols[j].Name })
	sort.Slice(a.Statuses, func(i, j int) bool { return a.Statuses[i].Name < a.Statuses[j].Name })
	sort.Slice(a.Services, func(i, j int) bool { return a.Services[i].Name < a.Services[j].Name })
	sort.Slice(a.Sources, func(i, j int) bool { return a.Sources[i].Path < a.Sources[j].Path })
}

func (a Artifact) Validate() error {
	if a.SchemaVersion != SchemaVersion {
		return fmt.Errorf("unsupported artifact schema %d", a.SchemaVersion)
	}
	protocolNames := make(map[string]bool)
	protocolIDs := make(map[uint32]string)
	for _, protocol := range a.Protocols {
		if protocol.Name == "" || protocolNames[protocol.Name] {
			return fmt.Errorf("duplicate or empty protocol name %q", protocol.Name)
		}
		protocolNames[protocol.Name] = true
		if old, ok := protocolIDs[protocol.UniqueID]; ok {
			return fmt.Errorf("protocol ID %#x is shared by %s and %s", protocol.UniqueID, old, protocol.Name)
		}
		protocolIDs[protocol.UniqueID] = protocol.Name
		positions := make(map[uint32]string)
		names := make(map[string]bool)
		for _, method := range protocol.Methods {
			if method.Name == "" || method.ResultType == "" || names[method.Name] {
				return fmt.Errorf("duplicate or empty method %s.%s", protocol.Name, method.Name)
			}
			names[method.Name] = true
			if old, ok := positions[method.Position]; ok {
				return fmt.Errorf("position %d is shared by %s.%s and %s.%s", method.Position, protocol.Name, old, protocol.Name, method.Name)
			}
			positions[method.Position] = method.Name
		}
	}
	for kind, values := range map[string][]NamedValue{"status": a.Statuses, "service": a.Services} {
		names := make(map[string]bool)
		for _, value := range values {
			if value.Name == "" || names[value.Name] {
				return fmt.Errorf("duplicate or empty %s name %q", kind, value.Name)
			}
			names[value.Name] = true
		}
	}
	sources := make(map[string]bool)
	for _, source := range a.Sources {
		if source.Path == "" || sources[source.Path] {
			return fmt.Errorf("duplicate or empty source path %q", source.Path)
		}
		if !hexDigest(source.SHA256) || !hexDigest(source.SemanticSHA256) {
			return fmt.Errorf("source %s has an invalid digest", source.Path)
		}
		sources[source.Path] = true
	}
	return nil
}

func hexDigest(value string) bool {
	if len(value) != 64 {
		return false
	}
	for _, character := range value {
		if !((character >= '0' && character <= '9') || (character >= 'a' && character <= 'f')) {
			return false
		}
	}
	return true
}

func ReadArtifact(path string) (Artifact, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Artifact{}, err
	}
	var artifact Artifact
	if err := json.Unmarshal(data, &artifact); err != nil {
		return Artifact{}, err
	}
	if err := artifact.Validate(); err != nil {
		return Artifact{}, err
	}
	return artifact, nil
}

func WriteArtifact(path string, artifact Artifact) error {
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
