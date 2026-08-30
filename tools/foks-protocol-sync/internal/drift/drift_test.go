package drift

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/aka-proj/foks-protocol-sync/internal/model"
)

func TestCompareClassifiesWireBehaviorAndAdditiveChanges(t *testing.T) {
	baseline := fixtureArtifact()
	candidate := fixtureArtifact()
	candidate.Source.Version = "mainline"
	candidate.Protocols[0].UniqueID++
	candidate.Protocols[0].Methods = append(candidate.Protocols[0].Methods, model.Method{
		Name: "added", Position: 9, QualifiedName: "Probe.added", ResultType: "void",
	})
	candidate.Sources[0].SHA256 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
	candidate.Sources[0].SemanticSHA256 = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
	report := Compare(baseline, candidate, Policy{
		Protocols: map[string]string{"Probe": "Probe"},
		Supported: map[string]bool{"Probe.probe": true},
	})
	if report.Counts[WireBreaking] != 1 {
		t.Fatalf("wire-breaking count = %d, changes = %#v", report.Counts[WireBreaking], report.Changes)
	}
	if report.Counts[BehaviorReviewRequired] != 1 {
		t.Fatalf("review count = %d, changes = %#v", report.Counts[BehaviorReviewRequired], report.Changes)
	}
	if report.Counts[Additive] != 1 {
		t.Fatalf("additive count = %d, changes = %#v", report.Counts[Additive], report.Changes)
	}
}

func TestCompareMarksUnsupportedMethodRemovalOutsideSlice(t *testing.T) {
	baseline := fixtureArtifact()
	baseline.Protocols[0].Methods = append(baseline.Protocols[0].Methods, model.Method{
		Name: "legacy", Position: 7, QualifiedName: "Probe.legacy", ResultType: "void",
	})
	report := Compare(baseline, fixtureArtifact(), Policy{
		Protocols: map[string]string{"Probe": "Probe"},
		Supported: map[string]bool{"Probe.probe": true},
	})
	if report.Counts[OutsideLocalSlice] != 1 {
		t.Fatalf("outside-slice count = %d, changes = %#v", report.Counts[OutsideLocalSlice], report.Changes)
	}
}

func TestCompareClassifiesFormattingOnlySnowpackChangeAsSourceOnly(t *testing.T) {
	baseline := fixtureArtifact()
	candidate := fixtureArtifact()
	candidate.Sources[0].SHA256 = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
	report := Compare(baseline, candidate, Policy{
		Protocols: map[string]string{"Probe": "Probe"},
		Supported: map[string]bool{"Probe.probe": true},
	})
	if report.Counts[SourceOnly] != 1 || report.Counts[BehaviorReviewRequired] != 0 {
		t.Fatalf("unexpected source classification: %#v", report.Changes)
	}
}

func TestCompareReviewsSupportedAndUnmappedRemSources(t *testing.T) {
	baseline := fixtureArtifact()
	baseline.Sources = append(baseline.Sources, model.SourceFile{
		Path: "proto-src/rem/invite.snowp", SHA256: strings.Repeat("a", 64),
		SemanticSHA256: strings.Repeat("b", 64),
	})
	candidate := baseline
	candidate.Sources = append([]model.SourceFile(nil), baseline.Sources...)
	candidate.Sources[1].SHA256 = strings.Repeat("d", 64)
	candidate.Sources[1].SemanticSHA256 = strings.Repeat("c", 64)
	report := Compare(baseline, candidate, Policy{
		Protocols: map[string]string{"Probe": "Probe"},
		Supported: map[string]bool{"Probe.probe": true},
	})
	if report.Counts[BehaviorReviewRequired] != 1 {
		t.Fatalf("unmapped rem source did not require review: %#v", report.Changes)
	}
}

func TestCompareClassifiesLocallyUsedNumericChangesAsWireBreaking(t *testing.T) {
	baseline := fixtureArtifact()
	candidate := fixtureArtifact()
	candidate.Protocols[0].Methods[0].Position = 2
	candidate.Statuses[0].Value = 1
	candidate.Services[0].Value = 11
	report := Compare(baseline, candidate, Policy{
		Protocols: map[string]string{"Probe": "Probe"},
		Supported: map[string]bool{"Probe.probe": true},
		Statuses:  map[string]bool{"OK": true},
		Services:  map[string]bool{"Probe": true},
		Coverage:  map[string][]string{"Probe.probe": {"probe_and_pin"}},
	})
	if report.Counts[WireBreaking] != 3 {
		t.Fatalf("wire-breaking count = %d, changes = %#v", report.Counts[WireBreaking], report.Changes)
	}
	foundCoverage := false
	for _, change := range report.Changes {
		if change.Kind == "method_position_changed" && strings.Contains(change.Detail, "coverage=probe_and_pin") {
			foundCoverage = true
		}
	}
	if !foundCoverage {
		t.Fatalf("method drift omitted coverage: %#v", report.Changes)
	}
}

func TestCompareKeepsUnsupportedProtocolNumericDriftOutsideSlice(t *testing.T) {
	baseline := fixtureArtifact()
	baseline.Protocols = append(baseline.Protocols, model.Protocol{
		Name: "Git", UniqueID: 22, GoFile: "proto/rem/git.go",
		Methods: []model.Method{{Name: "fetch", Position: 1, QualifiedName: "Git.fetch", ResultType: "GitPack"}},
	})
	candidate := baseline
	candidate.Protocols = append([]model.Protocol(nil), baseline.Protocols...)
	candidate.Protocols[1].UniqueID = 23
	candidate.Protocols[1].Methods = []model.Method{{Name: "pull", Position: 1, QualifiedName: "Git.pull", ResultType: "GitPack"}}
	report := Compare(baseline, candidate, Policy{
		Protocols: map[string]string{"Probe": "Probe"},
		Supported: map[string]bool{"Probe.probe": true},
	})
	if report.Counts[WireBreaking] != 0 || report.Counts[OutsideLocalSlice] != 3 {
		t.Fatalf("unexpected unsupported protocol classification: %#v", report.Changes)
	}
	if report.Counts[Additive] != 1 {
		t.Fatalf("renamed unsupported method should remain visible as additive: %#v", report.Changes)
	}
}

func TestCompareSerializesNoChangesAsAnEmptyArray(t *testing.T) {
	report := Compare(fixtureArtifact(), fixtureArtifact(), Policy{})
	encoded, err := json.Marshal(report)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(encoded), `"changes":[]`) {
		t.Fatalf("empty changes violate the report schema: %s", encoded)
	}
}

func TestCompareClassifiesHeaderAndResultDriftAsWireBreaking(t *testing.T) {
	baseline := fixtureArtifact()
	candidate := fixtureArtifact()
	candidate.Protocols[0].ArgumentHeader = false
	candidate.Protocols[0].ResultHeader = false
	candidate.Protocols[0].Methods[0].ResultType = "lib.SignedMerkleRoot"
	report := Compare(baseline, candidate, Policy{
		Protocols: map[string]string{"Probe": "Probe"},
		Supported: map[string]bool{"Probe.probe": true},
	})
	if report.Counts[WireBreaking] != 3 {
		t.Fatalf("wire-breaking count = %d, changes = %#v", report.Counts[WireBreaking], report.Changes)
	}
}

func fixtureArtifact() model.Artifact {
	return model.Artifact{
		SchemaVersion: model.SchemaVersion,
		Source:        model.SourceIdentity{Module: "github.com/foks-proj/go-foks", Version: "v0.1.9"},
		Protocols: []model.Protocol{{
			Name: "Probe", UniqueID: 1, GoFile: "proto/rem/probe.go",
			ArgumentHeader: true, ResultHeader: true,
			Methods: []model.Method{{Name: "probe", Position: 1, QualifiedName: "Probe.probe", ResultType: "rem.ProbeRes"}},
		}},
		Statuses: []model.NamedValue{{Name: "OK", Value: 0, GoFile: "proto/lib/status.go"}},
		Services: []model.NamedValue{{Name: "Probe", Value: 10, GoFile: "proto/lib/common.go"}},
		Sources: []model.SourceFile{{
			Path:           "proto-src/rem/probe.snowp",
			SHA256:         "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			SemanticSHA256: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
		}},
	}
}
