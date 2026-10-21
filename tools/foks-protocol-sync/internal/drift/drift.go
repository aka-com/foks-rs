package drift

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"sort"
	"strconv"
	"strings"

	"github.com/aka-com/foks-rs/tools/foks-protocol-sync/internal/model"
)

type Class string

const (
	WireBreaking           Class = "wire_breaking"
	BehaviorReviewRequired Class = "behavior_review_required"
	Additive               Class = "additive"
	OutsideLocalSlice      Class = "outside_local_slice"
	SourceOnly             Class = "source_only"
)

type Change struct {
	Class   Class  `json:"class"`
	Kind    string `json:"kind"`
	Subject string `json:"subject"`
	Detail  string `json:"detail"`
}

type Report struct {
	SchemaVersion int                  `json:"schema_version"`
	Baseline      model.SourceIdentity `json:"baseline"`
	Candidate     model.SourceIdentity `json:"candidate"`
	Counts        map[Class]int        `json:"counts"`
	Changes       []Change             `json:"changes"`
}

type Policy struct {
	Protocols map[string]string
	Supported map[string]bool
	Statuses  map[string]bool
	Services  map[string]bool
	Coverage  map[string][]string
	// Accepted lists, per supported route, result types from a newer
	// upstream release that the Rust client and server already handle
	// (`accepted_upstream_results` in the policy).
	Accepted map[string][]string
	// Tolerant maps an upstream struct (`rem.RTSendRes`) to how the Rust
	// decoder treats fields appended to it (`retained` or `read_only`), from
	// the policy's `[[tolerant_struct]]` tables. Growth of a listed struct is
	// additive; growth of any other struct needs review.
	Tolerant map[string]string
}

// ShapePair holds the struct-shape artifacts of the baseline and candidate.
type ShapePair struct {
	Baseline  model.ShapeArtifact
	Candidate model.ShapeArtifact
}

func ReadPolicy(path string) (Policy, error) {
	file, err := os.Open(path)
	if err != nil {
		return Policy{}, err
	}
	defer file.Close()
	policy := Policy{
		Protocols: make(map[string]string),
		Supported: make(map[string]bool),
		Statuses:  make(map[string]bool),
		Services:  make(map[string]bool),
		Coverage:  make(map[string][]string),
		Accepted:  make(map[string][]string),
		Tolerant:  make(map[string]string),
	}
	type block struct {
		kind, name, upstream, protocol, method string
		decoding                               string
		supported                              bool
		hasSupported                           bool
		coverage                               []string
		accepted                               []string
	}
	var current block
	flush := func() {
		switch current.kind {
		case "protocol":
			if current.name != "" && current.upstream != "" {
				policy.Protocols[current.name] = current.upstream
			}
		case "route":
			if current.hasSupported && current.supported {
				protocol := policy.Protocols[current.protocol]
				if protocol == "" {
					protocol = current.protocol
				}
				key := protocol + "." + current.method
				policy.Supported[key] = true
				policy.Coverage[key] = current.coverage
				if len(current.accepted) != 0 {
					policy.Accepted[key] = current.accepted
				}
			}
		case "service":
			if current.upstream != "" {
				policy.Services[current.upstream] = true
			}
		case "post_baseline_status":
			if current.upstream != "" {
				policy.Statuses[current.upstream] = true
			}
		case "tolerant_struct":
			if current.upstream != "" {
				policy.Tolerant[current.upstream] = current.decoding
			}
		}
		current = block{}
	}
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		line := strings.TrimSpace(strings.SplitN(scanner.Text(), "#", 2)[0])
		if line == "[status_aliases]" || line == "[[protocol]]" || line == "[[service]]" || line == "[[route]]" || line == "[[post_baseline_status]]" || line == "[[tolerant_struct]]" {
			flush()
			current.kind = strings.Trim(line, "[]")
			continue
		}
		key, value, ok := strings.Cut(line, "=")
		if !ok {
			continue
		}
		key, value = strings.TrimSpace(key), strings.TrimSpace(value)
		if current.kind == "status_aliases" {
			policy.Statuses[unquote(value)] = true
			continue
		}
		switch key {
		case "name":
			current.name = unquote(value)
		case "upstream":
			current.upstream = unquote(value)
		case "protocol":
			current.protocol = unquote(value)
		case "upstream_method":
			current.method = unquote(value)
		case "method":
			if current.method == "" {
				current.method = unquote(value)
			}
		case "supported":
			current.supported, _ = strconv.ParseBool(value)
			current.hasSupported = true
		case "coverage":
			current.coverage = stringArray(value)
		case "accepted_upstream_results":
			current.accepted = stringArray(value)
		case "decoding":
			current.decoding = unquote(value)
		}
	}
	flush()
	if err := scanner.Err(); err != nil {
		return Policy{}, err
	}
	if len(policy.Protocols) == 0 || len(policy.Supported) == 0 || len(policy.Statuses) == 0 || len(policy.Services) == 0 {
		return Policy{}, fmt.Errorf("policy contains no protocol, route, status, or service mapping")
	}
	for route := range policy.Supported {
		if len(policy.Coverage[route]) == 0 {
			return Policy{}, fmt.Errorf("supported route %s has no coverage ID", route)
		}
	}
	for name, decoding := range policy.Tolerant {
		if decoding != "retained" && decoding != "read_only" {
			return Policy{}, fmt.Errorf("tolerant struct %s has decoding %q, want retained or read_only", name, decoding)
		}
	}
	return policy, nil
}

func Compare(baseline, candidate model.Artifact, policy Policy) Report {
	return CompareWithShapes(baseline, candidate, nil, policy)
}

// CompareWithShapes is Compare plus, when shapes is non-nil, a per-struct
// comparison of positional field layouts.
func CompareWithShapes(baseline, candidate model.Artifact, shapes *ShapePair, policy Policy) Report {
	report := Report{
		SchemaVersion: 1, Baseline: baseline.Source, Candidate: candidate.Source,
		Changes: make([]Change, 0),
		Counts: map[Class]int{
			WireBreaking: 0, BehaviorReviewRequired: 0, Additive: 0,
			OutsideLocalSlice: 0, SourceOnly: 0,
		},
	}
	add := func(class Class, kind, subject, detail string) {
		report.Changes = append(report.Changes, Change{Class: class, Kind: kind, Subject: subject, Detail: detail})
		report.Counts[class]++
	}
	supportedProtocols := make(map[string]bool)
	for route := range policy.Supported {
		protocol, _, _ := strings.Cut(route, ".")
		supportedProtocols[protocol] = true
	}

	baseProtocols := protocols(baseline)
	candidateProtocols := protocols(candidate)
	for name, old := range baseProtocols {
		newProtocol, ok := candidateProtocols[name]
		if !ok {
			class := OutsideLocalSlice
			if supportedProtocols[name] {
				class = WireBreaking
			}
			add(class, "protocol_removed", name, "protocol is absent from candidate")
			continue
		}
		if old.UniqueID != newProtocol.UniqueID {
			class := OutsideLocalSlice
			if supportedProtocols[name] {
				class = WireBreaking
			}
			add(class, "protocol_id_changed", name, fmt.Sprintf("%#x -> %#x", old.UniqueID, newProtocol.UniqueID))
		}
		if old.ArgumentHeader != newProtocol.ArgumentHeader {
			class := OutsideLocalSlice
			if supportedProtocols[name] {
				class = WireBreaking
			}
			add(class, "argument_header_changed", name, fmt.Sprintf("%t -> %t", old.ArgumentHeader, newProtocol.ArgumentHeader))
		}
		if old.ResultHeader != newProtocol.ResultHeader {
			class := OutsideLocalSlice
			if supportedProtocols[name] {
				class = WireBreaking
			}
			add(class, "result_header_changed", name, fmt.Sprintf("%t -> %t", old.ResultHeader, newProtocol.ResultHeader))
		}
		compareMethods(old, newProtocol, policy, supportedProtocols[name], add)
	}
	for name := range candidateProtocols {
		if _, ok := baseProtocols[name]; !ok {
			add(Additive, "protocol_added", name, "new protocol")
		}
	}
	baseIDs := make(map[uint32]string)
	for name, protocol := range baseProtocols {
		baseIDs[protocol.UniqueID] = name
	}
	for name, protocol := range candidateProtocols {
		if old, ok := baseIDs[protocol.UniqueID]; ok && old != name {
			class := OutsideLocalSlice
			if supportedProtocols[old] || supportedProtocols[name] {
				class = WireBreaking
			}
			add(class, "protocol_id_reused", name, fmt.Sprintf("%#x previously belonged to %s", protocol.UniqueID, old))
		}
	}

	compareNamed("status", baseline.Statuses, candidate.Statuses, policy.Statuses, add)
	compareNamed("service", baseline.Services, candidate.Services, policy.Services, add)
	sourceOwners := protocolSourceOwners(baseline.Protocols, candidate.Protocols)
	compareSources(baseline.Sources, candidate.Sources, policy, supportedProtocols, sourceOwners, add)
	if shapes != nil {
		compareShapes(shapes.Baseline, shapes.Candidate, policy, supportedProtocols, sourceOwners, add)
	}
	sort.Slice(report.Changes, func(i, j int) bool {
		if report.Changes[i].Class != report.Changes[j].Class {
			return report.Changes[i].Class < report.Changes[j].Class
		}
		if report.Changes[i].Kind != report.Changes[j].Kind {
			return report.Changes[i].Kind < report.Changes[j].Kind
		}
		return report.Changes[i].Subject < report.Changes[j].Subject
	})
	return report
}

func compareMethods(old, candidate model.Protocol, policy Policy, protocolSupported bool, add func(Class, string, string, string)) {
	oldByName, newByName := make(map[string]model.Method), make(map[string]model.Method)
	oldByPosition, newByPosition := make(map[uint32]string), make(map[uint32]string)
	for _, method := range old.Methods {
		oldByName[method.Name], oldByPosition[method.Position] = method, method.Name
	}
	for _, method := range candidate.Methods {
		newByName[method.Name], newByPosition[method.Position] = method, method.Name
	}
	for name, method := range oldByName {
		subject := old.Name + "." + name
		newMethod, ok := newByName[name]
		class := OutsideLocalSlice
		if policy.Supported[subject] {
			class = WireBreaking
		}
		if !ok {
			add(class, "method_removed", subject, withCoverage(fmt.Sprintf("position %d is absent", method.Position), subject, policy))
		} else {
			if method.Position != newMethod.Position {
				add(class, "method_position_changed", subject, withCoverage(fmt.Sprintf("%d -> %d", method.Position, newMethod.Position), subject, policy))
			}
			if method.ResultType != newMethod.ResultType {
				if class == WireBreaking && accepted(policy.Accepted[subject], newMethod.ResultType) {
					add(Additive, "method_result_accepted", subject, withCoverage(fmt.Sprintf("%s -> %s; accepted by policy", method.ResultType, newMethod.ResultType), subject, policy))
				} else {
					add(class, "method_result_changed", subject, withCoverage(fmt.Sprintf("%s -> %s", method.ResultType, newMethod.ResultType), subject, policy))
				}
			}
		}
	}
	for name, method := range newByName {
		if _, ok := oldByName[name]; !ok {
			add(Additive, "method_added", candidate.Name+"."+name, fmt.Sprintf("position %d", method.Position))
		}
	}
	for position, oldName := range oldByPosition {
		if newName, ok := newByPosition[position]; ok && newName != oldName {
			class := OutsideLocalSlice
			if protocolSupported {
				class = WireBreaking
			}
			add(class, "method_position_reused", candidate.Name, fmt.Sprintf("position %d changed from %s to %s", position, oldName, newName))
		}
	}
}

func accepted(results []string, result string) bool {
	for _, candidate := range results {
		if candidate == result {
			return true
		}
	}
	return false
}

func compareNamed(kind string, old, candidate []model.NamedValue, relevant map[string]bool, add func(Class, string, string, string)) {
	oldValues, newValues := make(map[string]int64), make(map[string]int64)
	for _, value := range old {
		oldValues[value.Name] = value.Value
	}
	for _, value := range candidate {
		newValues[value.Name] = value.Value
	}
	for name, value := range oldValues {
		candidateValue, ok := newValues[name]
		class := OutsideLocalSlice
		if relevant[name] {
			class = WireBreaking
		}
		if !ok {
			add(class, kind+"_removed", name, fmt.Sprintf("value %d is absent", value))
		} else if value != candidateValue {
			add(class, kind+"_value_changed", name, fmt.Sprintf("%d -> %d", value, candidateValue))
		}
	}
	for name, value := range newValues {
		if _, ok := oldValues[name]; !ok {
			add(Additive, kind+"_added", name, fmt.Sprintf("value %d", value))
		}
	}
}

func compareSources(old, candidate []model.SourceFile, policy Policy, supported map[string]bool, owners map[string]map[string]bool, add func(Class, string, string, string)) {
	oldSources, newSources := make(map[string]model.SourceFile), make(map[string]model.SourceFile)
	for _, source := range old {
		oldSources[source.Path] = source
	}
	for _, source := range candidate {
		newSources[source.Path] = source
	}
	for path, source := range oldSources {
		newSource, ok := newSources[path]
		if ok && source.SHA256 == newSource.SHA256 {
			continue
		}
		class := SourceOnly
		if (!ok || source.SemanticSHA256 != newSource.SemanticSHA256) && sourceRelevant(path, supported, owners) {
			class = BehaviorReviewRequired
		}
		detail := "source removed"
		if ok {
			detail = source.SHA256 + " -> " + newSource.SHA256
			if source.SemanticSHA256 == newSource.SemanticSHA256 {
				detail += "; normalized Snowpack unchanged"
			}
		}
		if class == BehaviorReviewRequired {
			detail += coverageForSource(path, policy, owners)
		}
		add(class, "source_changed", path, detail)
	}
	for path, source := range newSources {
		if _, ok := oldSources[path]; !ok {
			class := SourceOnly
			if sourceRelevant(path, supported, owners) {
				class = BehaviorReviewRequired
			}
			detail := source.SHA256
			if class == BehaviorReviewRequired {
				detail += coverageForSource(path, policy, owners)
			}
			add(class, "source_added", path, detail)
		}
	}
}

// compareShapes classifies changes to positional struct layouts:
//
//   - struct_grown: the candidate appends fields to an unchanged prefix. go-foks
//     peers ignore unknown trailing fields; Rust decoders follow the policy in
//     crates/foks-proto/src/trailing.rs. Growth is additive for a struct the
//     policy lists in [[tolerant_struct]] and needs review otherwise, because
//     an exact Rust decoder rejects the longer form.
//   - struct_changed: an existing field was removed, renamed, retyped or
//     moved. That changes the meaning of positions already on the wire.
//   - struct_removed and struct_added: the type itself disappeared or appeared.
//
// A policy entry naming a struct the baseline does not define is reported so
// that a misspelt name cannot silently leave growth unclassified.
func compareShapes(old, candidate model.ShapeArtifact, policy Policy, supported map[string]bool, owners map[string]map[string]bool, add func(Class, string, string, string)) {
	oldShapes, newShapes := old.Shapes(), candidate.Shapes()
	relevant := func(shape model.StructShape) bool {
		return sourceRelevant(shapeSource(shape.GoFile), supported, owners)
	}
	for name, shape := range oldShapes {
		newShape, ok := newShapes[name]
		if !ok {
			class := OutsideLocalSlice
			if relevant(shape) {
				class = BehaviorReviewRequired
			}
			add(class, "struct_removed", name, fmt.Sprintf("%d fields; type is absent from candidate", len(shape.Fields)))
			continue
		}
		prefix := len(newShape.Fields) >= len(shape.Fields)
		for i := 0; prefix && i < len(shape.Fields); i++ {
			prefix = shape.Fields[i] == newShape.Fields[i]
		}
		switch {
		case prefix && len(newShape.Fields) == len(shape.Fields):
		case prefix:
			var appended []string
			for _, field := range newShape.Fields[len(shape.Fields):] {
				appended = append(appended, field.Name)
			}
			detail := fmt.Sprintf("%d -> %d fields; appended %s", len(shape.Fields), len(newShape.Fields), strings.Join(appended, ", "))
			if decoding, ok := policy.Tolerant[name]; ok {
				add(Additive, "struct_grown_tolerated", name, detail+"; Rust decoder is tolerant ("+decoding+")")
			} else {
				class := OutsideLocalSlice
				if relevant(shape) {
					class = BehaviorReviewRequired
				}
				add(class, "struct_grown", name, detail)
			}
		default:
			class := OutsideLocalSlice
			if relevant(shape) {
				class = WireBreaking
			}
			add(class, "struct_changed", name, fmt.Sprintf("%s -> %s", fieldList(shape.Fields), fieldList(newShape.Fields)))
		}
	}
	for name, shape := range newShapes {
		if _, ok := oldShapes[name]; !ok {
			add(Additive, "struct_added", name, fmt.Sprintf("%d fields", len(shape.Fields)))
		}
	}
	for name := range policy.Tolerant {
		if _, ok := oldShapes[name]; !ok {
			add(BehaviorReviewRequired, "tolerant_struct_unknown", name, "listed in [[tolerant_struct]] but absent from the baseline shapes")
		}
	}
}

// shapeSource maps a generated Go file to the Snowpack source it came from.
func shapeSource(goFile string) string {
	path := strings.TrimSuffix(goFile, ".go") + ".snowp"
	path = strings.Replace(path, "proto/lib/", "proto-src/lib/", 1)
	return strings.Replace(path, "proto/rem/", "proto-src/rem/", 1)
}

func fieldList(fields []model.StructField) string {
	names := make([]string, len(fields))
	for i, field := range fields {
		names[i] = field.Name + " " + field.Type
	}
	return "(" + strings.Join(names, ", ") + ")"
}

func sourceRelevant(path string, supported map[string]bool, owners map[string]map[string]bool) bool {
	if strings.HasPrefix(path, "proto-src/lib/") {
		return true
	}
	protocols, mapped := owners[path]
	for protocol := range protocols {
		if supported[protocol] {
			return true
		}
	}
	// An unmapped rem source can still define argument types consumed by a
	// supported protocol (invite.snowp is the v0.1.9 example). Treat it as
	// review-required instead of silently classifying it outside the slice.
	return !mapped && strings.HasPrefix(path, "proto-src/rem/")
}

func protocolSourceOwners(artifacts ...[]model.Protocol) map[string]map[string]bool {
	result := make(map[string]map[string]bool)
	for _, protocols := range artifacts {
		for _, protocol := range protocols {
			path := strings.TrimSuffix(protocol.GoFile, ".go")
			path = strings.Replace(path, "proto/rem/", "proto-src/rem/", 1) + ".snowp"
			if !strings.HasPrefix(path, "proto-src/rem/") {
				continue
			}
			if result[path] == nil {
				result[path] = make(map[string]bool)
			}
			result[path][protocol.Name] = true
		}
	}
	return result
}

func protocols(artifact model.Artifact) map[string]model.Protocol {
	result := make(map[string]model.Protocol)
	for _, protocol := range artifact.Protocols {
		result[protocol.Name] = protocol
	}
	return result
}

func unquote(value string) string {
	if result, err := strconv.Unquote(value); err == nil {
		return result
	}
	return value
}

func stringArray(value string) []string {
	value = strings.TrimSpace(value)
	if len(value) < 2 || value[0] != '[' || value[len(value)-1] != ']' {
		return nil
	}
	var result []string
	for _, entry := range strings.Split(value[1:len(value)-1], ",") {
		entry = strings.TrimSpace(entry)
		if entry != "" {
			result = append(result, unquote(entry))
		}
	}
	return result
}

func withCoverage(detail, subject string, policy Policy) string {
	if coverage := policy.Coverage[subject]; len(coverage) != 0 {
		return detail + "; coverage=" + strings.Join(coverage, ",")
	}
	return detail
}

func coverageForSource(path string, policy Policy, owners map[string]map[string]bool) string {
	var values []string
	seen := make(map[string]bool)
	for route, coverage := range policy.Coverage {
		protocol, _, _ := strings.Cut(route, ".")
		if !owners[path][protocol] {
			continue
		}
		for _, value := range coverage {
			if !seen[value] {
				seen[value] = true
				values = append(values, value)
			}
		}
	}
	sort.Strings(values)
	if len(values) == 0 {
		return ""
	}
	return "; coverage=" + strings.Join(values, ",")
}

func WriteJSON(path string, report Report) error {
	data, err := json.MarshalIndent(report, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, append(data, '\n'), 0o644)
}

func WriteMarkdown(path string, report Report) error {
	var output strings.Builder
	fmt.Fprintf(&output, "# FOKS protocol drift report\n\nBaseline: `%s` (`%s`)  \nCandidate: `%s` (`%s`)\n\n", report.Baseline.Version, report.Baseline.Commit, report.Candidate.Version, report.Candidate.Commit)
	if len(report.Changes) == 0 {
		output.WriteString("No normalized protocol drift detected.\n")
	} else {
		for _, change := range report.Changes {
			fmt.Fprintf(&output, "- **%s** `%s` `%s`: %s\n", change.Class, change.Kind, change.Subject, change.Detail)
		}
	}
	return os.WriteFile(path, []byte(output.String()), 0o644)
}
