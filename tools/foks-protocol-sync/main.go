package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"

	"github.com/aka-com/foks-rs/tools/foks-protocol-sync/internal/drift"
	"github.com/aka-com/foks-rs/tools/foks-protocol-sync/internal/extract"
	"github.com/aka-com/foks-rs/tools/foks-protocol-sync/internal/model"
)

const (
	modulePath     = "github.com/foks-proj/go-foks"
	pinnedVersion  = "v0.1.9"
	pinnedSum      = "h1:esVU4H00tL7kwyZXbPxfgTDJSBeqGRa5xPFv5xb/Ph0="
	pinnedGoModSum = "h1:x2wPw159s6fYDrvdpTCqqML392rkjqAniUCdkRIkudY="
)

type moduleDownload struct {
	Path, Version, Sum, GoModSum, Dir string
	Origin                            struct{ Hash string }
}

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, "foks-protocol-sync:", err)
		os.Exit(1)
	}
}

func run(arguments []string) error {
	if len(arguments) == 0 {
		return errors.New("usage: foks-protocol-sync <pinned|extract|diff> [flags]")
	}
	switch arguments[0] {
	case "pinned":
		return pinned(arguments[1:])
	case "extract":
		return extractLocal(arguments[1:])
	case "diff":
		return compare(arguments[1:])
	default:
		return fmt.Errorf("unknown command %q", arguments[0])
	}
}

func pinned(arguments []string) error {
	flags := flag.NewFlagSet("pinned", flag.ContinueOnError)
	oracle := flags.String("oracle", "tools/foks-v019-oracle", "directory containing the pinned oracle go.mod")
	out := flags.String("out", "", "output artifact")
	offline := flags.Bool("offline", false, "disable module network access")
	if err := flags.Parse(arguments); err != nil {
		return err
	}
	if *out == "" {
		return errors.New("--out is required")
	}
	command := exec.Command("go", "mod", "download", "-json", modulePath+"@"+pinnedVersion)
	command.Dir = *oracle
	if *offline {
		command.Env = append(os.Environ(), "GOPROXY=off")
	}
	data, err := command.Output()
	if err != nil {
		return fmt.Errorf("resolve pinned module: %w", err)
	}
	var downloaded moduleDownload
	if err := json.Unmarshal(data, &downloaded); err != nil {
		return err
	}
	if downloaded.Path != modulePath || downloaded.Version != pinnedVersion || downloaded.Sum != pinnedSum || downloaded.GoModSum != pinnedGoModSum {
		return fmt.Errorf("pinned module identity mismatch: path=%q version=%q sum=%q go_mod_sum=%q", downloaded.Path, downloaded.Version, downloaded.Sum, downloaded.GoModSum)
	}
	identity := model.SourceIdentity{Module: downloaded.Path, Version: downloaded.Version, Sum: downloaded.Sum, GoModSum: downloaded.GoModSum, Commit: downloaded.Origin.Hash}
	artifact, err := extract.Module(downloaded.Dir, identity)
	if err != nil {
		return err
	}
	return model.WriteArtifact(*out, artifact)
}

func extractLocal(arguments []string) error {
	flags := flag.NewFlagSet("extract", flag.ContinueOnError)
	dir := flags.String("module-dir", "", "checked-out go-foks source directory")
	version := flags.String("version", "mainline", "source version label")
	commit := flags.String("commit", "", "resolved source commit")
	out := flags.String("out", "", "output artifact")
	if err := flags.Parse(arguments); err != nil {
		return err
	}
	if *dir == "" || *out == "" {
		return errors.New("--module-dir and --out are required")
	}
	artifact, err := extract.Module(*dir, model.SourceIdentity{Module: modulePath, Version: *version, Commit: *commit})
	if err != nil {
		return err
	}
	return model.WriteArtifact(*out, artifact)
}

func compare(arguments []string) error {
	flags := flag.NewFlagSet("diff", flag.ContinueOnError)
	baselinePath := flags.String("baseline", "", "baseline metadata JSON")
	candidatePath := flags.String("candidate", "", "candidate metadata JSON")
	policyPath := flags.String("policy", "", "local policy TOML")
	jsonPath := flags.String("json", "", "JSON report output")
	markdownPath := flags.String("markdown", "", "Markdown report output")
	fail := flags.Bool("fail-on-review", false, "return failure for breaking or review-required drift")
	if err := flags.Parse(arguments); err != nil {
		return err
	}
	if *baselinePath == "" || *candidatePath == "" || *policyPath == "" {
		return errors.New("--baseline, --candidate, and --policy are required")
	}
	baseline, err := model.ReadArtifact(*baselinePath)
	if err != nil {
		return err
	}
	candidate, err := model.ReadArtifact(*candidatePath)
	if err != nil {
		return err
	}
	policy, err := drift.ReadPolicy(*policyPath)
	if err != nil {
		return err
	}
	report := drift.Compare(baseline, candidate, policy)
	if *jsonPath == "" {
		*jsonPath = filepath.Join(os.TempDir(), "foks-protocol-drift.json")
	}
	if *markdownPath == "" {
		*markdownPath = filepath.Join(os.TempDir(), "foks-protocol-drift.md")
	}
	if err := drift.WriteJSON(*jsonPath, report); err != nil {
		return err
	}
	if err := drift.WriteMarkdown(*markdownPath, report); err != nil {
		return err
	}
	if *fail && (report.Counts[drift.WireBreaking] != 0 || report.Counts[drift.BehaviorReviewRequired] != 0) {
		return fmt.Errorf("review-required protocol drift detected; see %s", *markdownPath)
	}
	return nil
}
