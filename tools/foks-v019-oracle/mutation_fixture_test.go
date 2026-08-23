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
	first := t.TempDir()
	second := t.TempDir()
	if err := writeMutationFixtures(first, userDir); err != nil {
		t.Fatal(err)
	}
	if err := writeMutationFixtures(second, userDir); err != nil {
		t.Fatal(err)
	}
	left := readFixtureTree(t, first)
	right := readFixtureTree(t, second)
	if len(left) != len(right) {
		t.Fatalf("fixture counts differ: %d != %d", len(left), len(right))
	}
	for name, expected := range left {
		actual, ok := right[name]
		if !ok {
			t.Fatalf("second fixture tree is missing %q", name)
		}
		if !bytes.Equal(actual, expected) {
			t.Fatalf("fixture %q changed across identical generations", name)
		}
	}
}
