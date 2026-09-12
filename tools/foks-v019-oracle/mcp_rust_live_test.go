package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"os"
	"os/exec"
	"strings"
	"testing"
	"time"

	"github.com/foks-proj/go-foks/proto/lcl"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// Uses the independent Go SDK and upstream JSON types against the packaged Rust
// stdio adapter. The scenarios mirror pinned integration-tests/cli/mcp_*_test.go.
func TestGoSDKAgainstRustMCP(t *testing.T) {
	cli, state := os.Getenv("FOKS_MCP_CLI"), os.Getenv("FOKS_MCP_STATE_DIR")
	if cli == "" || state == "" {
		t.Skip("run through run-mcp-compat.sh")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	connect := func(set string) *mcp.ClientSession {
		command := exec.Command(cli, "--state-dir", state, "mcp", set, "--profile", "local", "--account", "owner")
		command.Stderr = os.Stderr
		client := mcp.NewClient(&mcp.Implementation{Name: "go-mcp-oracle", Version: "v0.1.9"}, nil)
		session, err := client.Connect(ctx, &mcp.CommandTransport{Command: command}, nil)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { session.Close() })
		return session
	}
	kv := connect("kv")
	tools, err := kv.ListTools(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	names := make(map[string]bool)
	for _, tool := range tools.Tools {
		names[tool.Name] = true
	}
	for _, name := range []string{"list", "get", "stat", "usage", "put", "mkdir", "rm", "mv"} {
		if !names[name] {
			t.Fatalf("missing upstream tool %s", name)
		}
	}
	call := func(session *mcp.ClientSession, name string, args map[string]any, wantError bool) string {
		t.Helper()
		result, err := session.CallTool(ctx, &mcp.CallToolParams{Name: name, Arguments: args})
		if err != nil {
			t.Fatalf("%s protocol error: %v", name, err)
		}
		if result.IsError != wantError || len(result.Content) != 1 {
			t.Fatalf("%s unexpected outcome: %+v", name, result)
		}
		text, ok := result.Content[0].(*mcp.TextContent)
		if !ok {
			t.Fatalf("%s did not return text", name)
		}
		return text.Text
	}
	if got := call(kv, "put", map[string]any{"path": "sdk/a/file", "content": "hello", "mkdir_p": true}, false); got != "ok" {
		t.Fatal(got)
	}
	call(kv, "put", map[string]any{"path": "sdk/a/file", "content": "replacement"}, true)
	call(kv, "put", map[string]any{"path": "sdk/a/file", "content": "replacement", "overwrite": true}, false)
	if got := call(kv, "get", map[string]any{"path": "sdk/a/file"}, false); got != "replacement" {
		t.Fatal(got)
	}
	var root lcl.KVStat
	rootJSON := call(kv, "stat", map[string]any{"path": "/"}, false)
	if err := json.Unmarshal([]byte(rootJSON), &root); err != nil || root.De != nil || root.V.T != proto.KVNodeType_Dir {
		t.Fatalf("Go root stat: %v %s", err, rootJSON)
	}
	var stat lcl.KVStat
	encoded := call(kv, "stat", map[string]any{"path": "sdk/a/file"}, false)
	if err := json.Unmarshal([]byte(encoded), &stat); err != nil {
		t.Fatalf("Go KVStat JSON: %v", err)
	}
	typ, err := stat.V.GetT()
	if err != nil || typ != proto.KVNodeType_SmallFile || stat.V.Smallfile().Size != 11 || stat.De == nil {
		t.Fatalf("incorrect Go stat: %s", encoded)
	}
	if got := call(kv, "list", map[string]any{"path": "sdk/a"}, false); !strings.Contains(got, "file\tfile\t") {
		t.Fatal(got)
	}
	if got := call(kv, "mkdir", map[string]any{"path": "sdk/destination"}, false); !strings.HasPrefix(got, "DirID: 1") {
		t.Fatal(got)
	}
	if got := call(kv, "mv", map[string]any{"src": "sdk/a/file", "dst": "sdk/destination"}, false); got != "ok" {
		t.Fatal(got)
	}
	if got := call(kv, "get", map[string]any{"path": "sdk/destination/file"}, false); got != "replacement" {
		t.Fatal(got)
	}
	binary := base64.StdEncoding.EncodeToString([]byte{0, 1, 255, 128})
	call(kv, "put", map[string]any{"path": "sdk/binary", "content": binary, "base64": true}, false)
	call(kv, "get", map[string]any{"path": "sdk/binary"}, true)
	if got := call(kv, "get", map[string]any{"path": "sdk/binary", "base64": true}, false); got != binary {
		t.Fatal(got)
	}
	if got := call(kv, "usage", map[string]any{}, false); !strings.HasPrefix(got, "Num Files:") {
		t.Fatal(got)
	}
	call(kv, "put", map[string]any{"path": "sdk-team", "team": "mcpteam", "content": "team"}, false)
	if got := call(kv, "get", map[string]any{"path": "sdk-team", "team": "mcpteam"}, false); got != "team" {
		t.Fatal(got)
	}
	call(kv, "rm", map[string]any{"path": "sdk"}, true)
	if got := call(kv, "rm", map[string]any{"path": "sdk", "recursive": true}, false); got != "ok" {
		t.Fatal(got)
	}
	team := connect("team")
	if got := call(team, "list", map[string]any{"team": "mcpteam"}, false); !strings.Contains(got, "mcpowner\t-\to\to\t") {
		t.Fatal(got)
	}
	if got := call(team, "list-memberships", map[string]any{}, false); !strings.Contains(got, "mcpteam\t") {
		t.Fatal(got)
	}
	call(team, "list", map[string]any{"team": "missing-team"}, true)
}
