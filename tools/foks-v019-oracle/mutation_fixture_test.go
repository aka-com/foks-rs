package main

import (
	"bytes"
	"io/fs"
	"os"
	"path/filepath"
	"testing"
)

func readFixtureTree(t *testing.T, root string) map[string][]byte {
	t.Helper()
	ret := make(map[string][]byte)
	err := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() {
			return nil
		}
		relative, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		ret[relative] = data
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	return ret
}

func TestMutationFixturesAreByteReproducible(t *testing.T) {
	userDir := filepath.Join(
		"..", "..", "crates", "foks-snowpack", "tests", "fixtures", "foks-v0.1.9", "user",
	)
	committedDir := filepath.Join(
		"..", "..", "crates", "foks-snowpack", "tests", "fixtures", "foks-v0.1.9", "user-mutations",
	)
	generatedDir := t.TempDir()
	if err := writeMutationFixtures(generatedDir, userDir); err != nil {
		t.Fatal(err)
	}
	committed := readFixtureTree(t, committedDir)
	generated := readFixtureTree(t, generatedDir)
	if len(committed) != len(generated) {
		t.Fatalf("fixture counts differ: committed=%d generated=%d", len(committed), len(generated))
	}
	for name, expected := range committed {
		actual, ok := generated[name]
		if !ok {
			t.Fatalf("generated fixture tree is missing committed artifact %q", name)
		}
		if !bytes.Equal(actual, expected) {
			t.Fatalf("generated fixture %q differs from the committed oracle", name)
		}
	}
}
