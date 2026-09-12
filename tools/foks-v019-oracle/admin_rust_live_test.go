package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"regexp"
	"strings"
	"testing"
	"time"

	"github.com/foks-proj/go-foks/lib/core"
	proto "github.com/foks-proj/go-foks/proto/lib"
	"github.com/foks-proj/go-foks/proto/rem"
)

// Rust intentionally consumes a native ticket on browser confirmation. The real
// pinned client first issues and checks it through two separate RPC calls.
func TestGoAdminAgainstRustServer(t *testing.T) {
	probeAddress, caPath := os.Getenv("FOKS_GO_RUST_PROBE"), os.Getenv("FOKS_GO_RUST_CA_DER")
	if probeAddress == "" || os.Getenv("FOKS_GO_ADMIN_PROXY") == "" {
		t.Skip("run through Rust admin HTTPS gate")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	p, closeP := liveRPCClient(t, ctx, probeAddress, liveRootPool(t, caPath), nil)
	defer closeP()
	probeClient := core.NewProbeClient(p, nil)
	probe, err := probeClient.Probe(ctx, rem.ProbeArg{Hostname: proto.Hostname("localhost")})
	if err != nil {
		t.Fatal(err)
	}
	chain, err := core.PlayChain(proto.TCPAddr(probeAddress), probe.Hostchain, nil)
	if err != nil {
		t.Fatal(err)
	}
	zone, err := core.CheckZoneSig(*chain, probe)
	if err != nil {
		t.Fatal(err)
	}
	roots, err := chain.RootCACertPool()
	if err != nil {
		t.Fatal(err)
	}
	public, closePublic := liveRPCClient(t, ctx, string(zone.Services.Reg), roots, nil)
	defer closePublic()
	reg := core.NewRegClient(public, nil)
	merkle := core.NewMerkleQueryClient(public, nil)
	user := liveSignup(t, ctx, &reg, &merkle, chain.HostID())
	key, err := user.device.EntityID()
	if err != nil {
		t.Fatal(err)
	}
	cert, err := reg.GetClientCertChain(ctx, rem.GetClientCertChainArg{Uid: user.uid, Key: key})
	if err != nil {
		t.Fatal(err)
	}
	private, err := user.device.PrivateKeyForCert()
	if err != nil {
		t.Fatal(err)
	}
	auth, closeAuth := liveRPCClient(t, ctx, string(zone.Services.User), roots, &tls.Certificate{Certificate: cert, PrivateKey: private})
	defer closeAuth()
	client := core.NewUserClient(auth, nil)
	link, err := client.NewWebAdminPanelURL(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if err = client.CheckURL(ctx, link); err != nil {
		t.Fatalf("check before browser redemption: %v", err)
	}
	parsed, err := url.Parse(string(link))
	if err != nil {
		t.Fatal(err)
	}
	ticket, err := core.B62Decode(parsed.Query().Get("session"))
	if err != nil || len(ticket) != 20 {
		t.Fatal("ticket is not Go-compatible Base62")
	}
	certBytes, err := os.ReadFile(os.Getenv("FOKS_GO_ADMIN_CA_DER"))
	if err != nil {
		t.Fatal(err)
	}
	certRoot, err := x509.ParseCertificate(certBytes)
	if err != nil {
		t.Fatal(err)
	}
	browserRoots := x509.NewCertPool()
	browserRoots.AddCert(certRoot)
	transport := &http.Transport{TLSClientConfig: &tls.Config{RootCAs: browserRoots, MinVersion: tls.VersionTLS12}, DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
		return (&net.Dialer{Timeout: 5 * time.Second}).DialContext(ctx, network, os.Getenv("FOKS_GO_ADMIN_PROXY"))
	}}
	defer transport.CloseIdleConnections()
	browser := &http.Client{Transport: transport, Timeout: 10 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	request := func(method, path string, cookie *http.Cookie, body string) *http.Response {
		t.Helper()
		req, err := http.NewRequestWithContext(ctx, method, path, strings.NewReader(body))
		if err != nil {
			t.Fatal(err)
		}
		if cookie != nil {
			req.AddCookie(cookie)
		}
		if method == "POST" {
			req.Header.Set("Origin", parsed.Scheme+"://"+parsed.Host)
			req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		}
		r, err := browser.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		return r
	}
	staged := request("GET", string(link), nil, "")
	if staged.StatusCode != 303 {
		t.Fatalf("stage status %d", staged.StatusCode)
	}
	var pending *http.Cookie
	for _, c := range staged.Cookies() {
		if c.Name == "__Host-foks_admin_pending" {
			pending = c
		}
	}
	staged.Body.Close()
	if pending == nil {
		t.Fatal("missing pending cookie")
	}
	clean := parsed.Scheme + "://" + parsed.Host + "/login/confirm"
	confirmation := request("GET", clean, pending, "")
	body, err := io.ReadAll(io.LimitReader(confirmation.Body, 128*1024))
	confirmation.Body.Close()
	if err != nil {
		t.Fatal(err)
	}
	match := regexp.MustCompile(`name="csrf" value="([0-9a-f]{64})"`).FindSubmatch(body)
	if len(match) != 2 {
		t.Fatal("missing confirmation CSRF")
	}
	redeemed := request("POST", clean, pending, "csrf="+string(match[1]))
	redeemed.Body.Close()
	if redeemed.StatusCode != 303 {
		t.Fatalf("redeem status %d", redeemed.StatusCode)
	}
	err = client.CheckURL(ctx, link)
	var expired core.ExpiredError
	if !errors.As(err, &expired) {
		t.Fatalf("consumed ticket must be expired: %v", err)
	}
}
