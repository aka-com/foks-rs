package main

import (
	"context"
	"crypto/sha256"
	"crypto/x509"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"time"

	"github.com/foks-proj/go-foks/lib/core"
	"github.com/foks-proj/go-foks/lib/merkle"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
	"github.com/foks-proj/go-snowpack-rpc/rpc"
	"go.uber.org/zap"
)

type oracleContext struct {
	ctx context.Context
	log *zap.Logger
}

func (m *oracleContext) WithLogTag(string) core.RpcClientMetaContexter { return m }
func (m *oracleContext) Background() core.RpcClientMetaContexter {
	return &oracleContext{ctx: context.Background(), log: m.log}
}
func (m *oracleContext) Infow(string, ...interface{}) {}
func (m *oracleContext) Warnw(string, ...interface{}) {}
func (m *oracleContext) WarnwWithContext(context.Context, string, ...interface{}) {
}
func (m *oracleContext) RPCLogOptions() (rpc.LogOptions, error) {
	return &rpc.StandardLogOptions{}, nil
}
func (m *oracleContext) Ctx() context.Context                        { return m.ctx }
func (m *oracleContext) Log() *zap.Logger                            { return m.log }
func (m *oracleContext) CnameResolver() core.CNameResolver           { return nil }
func (m *oracleContext) NetworkConditioner() core.NetworkConditioner { return nil }

type fixture struct {
	File   string `json:"file"`
	Bytes  int    `json:"bytes"`
	SHA256 string `json:"sha256"`
}

type manifest struct {
	Format          string    `json:"format"`
	FOKSVersion     string    `json:"foks_version"`
	CapturedAt      string    `json:"captured_at"`
	ProbeAddress    string    `json:"probe_address"`
	HostID          string    `json:"host_id"`
	HostchainSeqno  uint64    `json:"hostchain_seqno"`
	HostchainTail   string    `json:"hostchain_tail"`
	MerkleEpoch     uint64    `json:"merkle_epoch"`
	MerkleRootNode  string    `json:"merkle_root_node"`
	MerkleRootHash  string    `json:"merkle_root_hash"`
	MerkleHostchain string    `json:"merkle_hostchain_tail"`
	Files           []fixture `json:"files"`
	RPCFiles        []fixture `json:"rpc_files"`
}

type writer struct {
	dir      string
	files    []fixture
	rpcFiles []fixture
}

func (w *writer) bytes(name string, data []byte) error {
	if err := core.AssertCanonicalMsgpack(data); err != nil {
		return fmt.Errorf("%s is not canonical Snowpack: %w", name, err)
	}
	return w.write(name, data, &w.files)
}

func (w *writer) raw(name string, data []byte) error {
	return w.write(name, data, &w.rpcFiles)
}

func (w *writer) write(name string, data []byte, fixtures *[]fixture) error {
	path := filepath.Join(w.dir, name)
	if err := os.WriteFile(path, data, 0o644); err != nil {
		return err
	}
	hash := sha256.Sum256(data)
	*fixtures = append(*fixtures, fixture{
		File:   name,
		Bytes:  len(data),
		SHA256: hex.EncodeToString(hash[:]),
	})
	return nil
}

func probeRequestFrame(address proto.TCPAddr) ([]byte, error) {
	arg := rem.ProbeArg{
		Hostname:           address.Hostname().Normalize(),
		HostchainLastSeqno: 0,
	}
	return rpcRequestFrame(rem.ProbeProtocolID, 1, arg.Export())
}

func (w *writer) object(name string, object core.Encodeable) error {
	data, err := core.EncodeToBytes(object)
	if err != nil {
		return fmt.Errorf("encode %s: %w", name, err)
	}
	return w.bytes(name, data)
}

func verifyMerkleRoot(chain *core.Hostchain, signed proto.SignedMerkleRoot) (*proto.MerkleRoot, error) {
	for _, key := range chain.Keys(proto.EntityType_HostMerkleSigner) {
		public, err := core.ImportEntityPublic(key)
		if err != nil {
			return nil, err
		}
		root, err := core.Verify2[*proto.MerkleRoot](public, signed.Sig, &signed.Inner)
		if err == nil {
			return root, nil
		}
	}
	return nil, errors.New("no delegated host Merkle key verified the signed root")
}

func capture(ctx context.Context, address proto.TCPAddr, timeout time.Duration) (rem.ProbeRes, error) {
	var zero rem.ProbeRes
	roots, err := x509.SystemCertPool()
	if err != nil {
		return zero, err
	}
	m := &oracleContext{ctx: ctx, log: zap.NewNop()}
	opts := core.NewRpcClientOpts()
	opts.Timeout = timeout
	opts.NumConnectAttempts = 1
	generic := core.NewRpcClient(m, address, roots, nil, opts)
	defer generic.Shutdown()
	client := core.NewProbeClient(generic, m)
	return client.Probe(ctx, rem.ProbeArg{
		Hostname:           address.Hostname().Normalize(),
		HostchainLastSeqno: 0,
	})
}

func run() error {
	var host string
	var output string
	var userOutput string
	var signupOutput string
	var mutationOutput string
	var mutationUserDir string
	var probeFile string
	var timeout time.Duration
	flag.StringVar(&host, "host", "foks.app:4430", "FOKS probe host and port")
	flag.StringVar(&output, "out", "", "fixture output directory")
	flag.StringVar(&userOutput, "user-out", "", "optional self-contained user/device fixture output directory")
	flag.StringVar(&signupOutput, "signup-out", "", "optional software-eldest signup fixture output directory")
	flag.StringVar(&mutationOutput, "mutation-out", "", "optional user-mutation fixture output directory")
	flag.StringVar(&mutationUserDir, "mutation-user-dir", "", "verified user fixture input directory")
	flag.StringVar(&probeFile, "probe-file", "", "use an existing canonical ProbeRes instead of the network")
	flag.DurationVar(&timeout, "timeout", 15*time.Second, "probe timeout")
	flag.Parse()
	if output == "" {
		return errors.New("--out is required")
	}
	if err := os.MkdirAll(output, 0o755); err != nil {
		return err
	}

	address := proto.TCPAddr(host)
	var response rem.ProbeRes
	if probeFile == "" {
		ctx, cancel := context.WithTimeout(context.Background(), timeout)
		defer cancel()
		var err error
		response, err = capture(ctx, address, timeout)
		if err != nil {
			return err
		}
	} else {
		data, err := os.ReadFile(probeFile)
		if err != nil {
			return err
		}
		if err := core.DecodeFromBytes(&response, data); err != nil {
			return fmt.Errorf("decode probe fixture: %w", err)
		}
	}
	chain, err := core.PlayChain(address, response.Hostchain, nil)
	if err != nil {
		return fmt.Errorf("verify hostchain: %w", err)
	}
	if _, err := core.CheckZoneSig(*chain, response); err != nil {
		return fmt.Errorf("verify public zone: %w", err)
	}
	root, err := verifyMerkleRoot(chain, response.MerkleRoot)
	if err != nil {
		return fmt.Errorf("verify Merkle root: %w", err)
	}
	if userOutput != "" {
		if err := writeUserFixtures(userOutput, address, chain.HostID(), root); err != nil {
			return fmt.Errorf("write user fixtures: %w", err)
		}
	}
	if signupOutput != "" {
		if err := writeSignupFixtures(signupOutput, chain.HostID(), root); err != nil {
			return fmt.Errorf("write signup fixtures: %w", err)
		}
	}
	if mutationOutput != "" {
		if mutationUserDir == "" {
			return errors.New("--mutation-user-dir is required with --mutation-out")
		}
		if err := writeMutationFixtures(mutationOutput, mutationUserDir); err != nil {
			return fmt.Errorf("write mutation fixtures: %w", err)
		}
	}
	rootVersion, err := root.GetV()
	if err != nil {
		return err
	}
	if rootVersion != proto.MerkleRootVersion_V1 {
		return fmt.Errorf("unsupported Merkle root version %d", rootVersion)
	}
	rootV1 := root.V1()
	if !chain.Tail().Eq(rootV1.Hostchain) {
		return errors.New("Merkle root does not commit to the verified hostchain tail")
	}
	var rootHash proto.MerkleRootHash
	if err := merkle.HashRoot(root, &rootHash); err != nil {
		return fmt.Errorf("hash Merkle root: %w", err)
	}
	state, err := chain.Export()
	if err != nil {
		return err
	}

	w := writer{dir: output}
	requestFrame, err := probeRequestFrame(address)
	if err != nil {
		return fmt.Errorf("encode probe RPC request: %w", err)
	}
	if err := w.raw("probe-request.frame", requestFrame); err != nil {
		return err
	}
	if err := w.object("probe-response.snowp", &response); err != nil {
		return err
	}
	if err := w.object("signed-merkle-root.snowp", &response.MerkleRoot); err != nil {
		return err
	}
	if err := w.bytes("merkle-root-inner.snowp", response.MerkleRoot.Inner.Bytes()); err != nil {
		return err
	}
	if err := w.object("signed-public-zone.snowp", &response.Zone); err != nil {
		return err
	}
	if err := w.bytes("public-zone-inner.snowp", response.Zone.Inner.Bytes()); err != nil {
		return err
	}
	if err := w.object("hostchain-state.snowp", &state); err != nil {
		return err
	}
	hostID := chain.HostID()
	if err := w.object("host-id.snowp", &hostID); err != nil {
		return err
	}
	linkHashes := make([]string, 0, len(response.Hostchain))
	for index := range response.Hostchain {
		link := &response.Hostchain[index]
		name := fmt.Sprintf("hostchain-link-%04d.snowp", index+1)
		if err := w.object(name, link); err != nil {
			return err
		}
		hash, err := core.HostchainLinkHash(link)
		if err != nil {
			return err
		}
		linkHashes = append(linkHashes, hash.String())
	}
	sort.Slice(w.files, func(i, j int) bool { return w.files[i].File < w.files[j].File })
	sort.Slice(w.rpcFiles, func(i, j int) bool { return w.rpcFiles[i].File < w.rpcFiles[j].File })

	tail := chain.Tail()
	result := manifest{
		Format:          "foks-v0.1.9-probe-fixtures-v2",
		FOKSVersion:     "v0.1.9",
		CapturedAt:      time.Now().UTC().Format(time.RFC3339),
		ProbeAddress:    host,
		HostID:          hostID.String(),
		HostchainSeqno:  uint64(tail.Seqno),
		HostchainTail:   tail.Hash.String(),
		MerkleEpoch:     uint64(rootV1.Epno),
		MerkleRootNode:  hex.EncodeToString(rootV1.RootNode[:]),
		MerkleRootHash:  hex.EncodeToString(rootHash[:]),
		MerkleHostchain: rootV1.Hostchain.Hash.String(),
		Files:           w.files,
		RPCFiles:        w.rpcFiles,
	}
	manifestBytes, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return err
	}
	manifestBytes = append(manifestBytes, '\n')
	if err := os.WriteFile(filepath.Join(output, "manifest.json"), manifestBytes, 0o644); err != nil {
		return err
	}
	for index, hash := range linkHashes {
		fmt.Printf("hostchain-link-%04d %s\n", index+1, hash)
	}
	fmt.Printf(
		"captured %d Snowpack and %d RPC fixtures for %s at Merkle epoch %d\n",
		len(w.files), len(w.rpcFiles), hostID, rootV1.Epno,
	)
	return nil
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "foks-v019-oracle:", err)
		os.Exit(1)
	}
}
