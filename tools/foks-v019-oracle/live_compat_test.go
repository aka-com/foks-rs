package main

import (
	"crypto/tls"
	"crypto/x509"
	"encoding/pem"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/foks-proj/go-foks/integration-tests/common"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/server/shared"
)

// TestRustClientHappyPath boots the unmodified v0.1.9 Go services and their
// Postgres databases, then delegates the client side of the transaction to the
// Rust executable supplied by run-live-compat.sh. It is opt-in so ordinary Go
// fixture tests remain hermetic and do not require Docker.
func TestRustClientHappyPath(t *testing.T) {
	driver := os.Getenv("FOKS_RUST_LIVE_DRIVER")
	if driver == "" {
		t.Skip("set FOKS_RUST_LIVE_DRIVER or use run-live-compat.sh")
	}

	environment := common.NewTestEnv()
	err := environment.Setup(common.SetupOpts{
		PrimaryHostname: "localhost",
		MerklePollWait:  time.Hour,
		Icr:             proto.InviteCodeRegime_CodeOptional,
	})
	if err != nil {
		t.Fatalf("start official v0.1.9 integration environment: %v", err)
	}
	// Setup's primary-host path does not apply Icr to host_config in v0.1.9;
	// use the same official helper as the upstream signup tests.
	if err := common.SetInviteCodeOptional(environment.MetaContext()); err != nil {
		t.Fatalf("make invite code optional: %v", err)
	}
	t.Cleanup(func() {
		if err := environment.Shutdown(); err != nil {
			t.Logf("stop official v0.1.9 integration environment: %v", err)
		}
	})
	// The upstream test helper deliberately uses a 1024-bit RSA probe leaf
	// when its inTest flag is set. Modern rustls providers correctly reject
	// that obsolete key size. Replace only the ephemeral public TLS leaf via
	// the same official helper's production-strength path; all handlers,
	// protocol state, CAs, and authenticated hostchain material remain those
	// of the unmodified v0.1.9 integration environment.
	_, err = shared.EmulateLetsEncrypt(
		environment.MetaContext(),
		[]proto.Hostname{"localhost", "127.0.0.1", "::1"},
		nil,
		environment.X509Material().ProbeCA.Cert,
		environment.X509Material().ProbeCA.Key,
		proto.CKSAssetType_RootPKIFrontendX509Cert,
		false,
	)
	if err != nil {
		t.Fatalf("install production-strength probe certificate: %v", err)
	}
	if err := environment.DirectMerklePoke(); err != nil {
		t.Fatalf("initialize official Merkle pipeline: %v", err)
	}

	certificatePEM, err := os.ReadFile(environment.X509Material().ProbeCA.CertFile.String())
	if err != nil {
		t.Fatalf("read probe CA: %v", err)
	}
	certificate, rest := pem.Decode(certificatePEM)
	if certificate == nil || certificate.Type != "CERTIFICATE" || len(rest) != 0 {
		t.Fatal("official probe CA file is not one PEM certificate")
	}
	probeCA, err := x509.ParseCertificate(certificate.Bytes)
	if err != nil {
		t.Fatalf("parse official probe CA: %v", err)
	}
	probeAddress := environment.ProbeSrv().ListenerAddr().String()
	probeHost, _, err := net.SplitHostPort(probeAddress)
	if err != nil {
		t.Fatalf("parse official probe address: %v", err)
	}
	rootCAs := x509.NewCertPool()
	rootCAs.AddCert(probeCA)
	probeTLS, err := tls.Dial("tcp", probeAddress, &tls.Config{
		RootCAs:    rootCAs,
		ServerName: probeHost,
		MinVersion: tls.VersionTLS12,
	})
	if err != nil {
		t.Fatalf("connect to official probe TLS: %v", err)
	}
	peerCertificates := probeTLS.ConnectionState().PeerCertificates
	if err := probeTLS.Close(); err != nil {
		t.Fatalf("close official probe TLS: %v", err)
	}
	if len(peerCertificates) == 0 {
		t.Fatal("official probe TLS returned no certificates")
	}
	t.Logf(
		"probe TLS leaf signature=%s public-key=%s CA signature=%s public-key=%s",
		peerCertificates[0].SignatureAlgorithm,
		peerCertificates[0].PublicKeyAlgorithm,
		probeCA.SignatureAlgorithm,
		probeCA.PublicKeyAlgorithm,
	)
	stateDirectory := t.TempDir()
	caDER := filepath.Join(stateDirectory, "probe-ca.der")
	if err := os.WriteFile(caDER, certificate.Bytes, 0o600); err != nil {
		t.Fatalf("write probe CA DER: %v", err)
	}

	stopPokes := make(chan struct{})
	pokeErrors := make(chan error, 1)
	var pokes sync.WaitGroup
	pokes.Add(1)
	go func() {
		defer pokes.Done()
		ticker := time.NewTicker(50 * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-stopPokes:
				return
			case <-ticker.C:
				if err := environment.DirectMerklePoke(); err != nil {
					select {
					case pokeErrors <- err:
					default:
					}
					return
				}
			}
		}
	}()

	username := "rustcompat"
	command := exec.Command(
		driver,
		"--probe", probeAddress,
		"--ca-der", caDER,
		"--state-dir", stateDirectory,
		"--username", username,
	)
	filteredStop := make(chan struct{})
	filteredDone := make(chan error, 1)
	if os.Getenv("FOKS_RUST_LIVE_CHAT") != "" {
		command.Env = append(os.Environ(), "FOKS_LIVE_FILTERED_DIR="+stateDirectory)
		go func() { filteredDone <- seedFilteredInbox(environment.MetaContext(), stateDirectory, filteredStop) }()
	}
	output, commandErr := command.CombinedOutput()
	close(filteredStop)
	if os.Getenv("FOKS_RUST_LIVE_CHAT") != "" {
		if err := <-filteredDone; err != nil {
			t.Errorf("filtered inbox fixture: %v", err)
		}
	}
	close(stopPokes)
	pokes.Wait()
	select {
	case err := <-pokeErrors:
		t.Fatalf("drive official Merkle pipeline: %v\n%s", err, output)
	default:
	}
	if commandErr != nil {
		t.Fatalf("Rust live compatibility driver: %v\n%s", commandErr, output)
	}
	t.Logf("%s", output)
}
